//! Floating-point instruction decoding: FPv4-SP (Cortex-M4F) and FPv5-D16 (Cortex-M7).
//!
//! Sources: ARM DDI 0403E.e (ARMv7-M ARM) chapter A6 (floating-point instruction encodings:
//! table A6-14 .. A6-17, "Floating-point data-processing", "Floating-point register transfers",
//! "Floating-point load/store") and chapter A7 (instruction pages VADD ... VSTM), Cortex-M7
//! Processor TRM (ARM DDI 0489) for the FPv5-D16 option and the encodings added by ARMv8
//! (VSEL, VMAXNM/VMINNM, VRINT*, VCVTA/N/P/M).
//!
//! Operand conventions (all in [`Insn`]):
//!
//! | op | fields |
//! |---|---|
//! | `VADD`..`VFNMS`, `VMAXNM`, `VMINNM` | `rd`, `rn`, `rm` (S index 0-31, or D index 0-15 when `aux` bit 0) |
//! | `VABS`/`VNEG`/`VSQRT`/`VMOV_F` | `rd`, `rm` |
//! | `VCMP`/`VCMPE` | `rn` = left register, `rm` = right register; `s` = 1: compare with +0.0 |
//! | `VMOV_I` | `rd`; `imm` = the 32 bits of a single, or the high 32 bits of a double |
//! | `VMOV_RS`/`VMOV_SR` | `rd` = core register, `rm` = S register |
//! | `VMOV_2S`/`VMOV_D2` | `rd` = Rt, `rn` = Rt2, `rm` = first S / D register; `aux` bit 0: FP -> core |
//! | `VMOV_SC` (`vmov.32 Dd[x], Rt`) | `rd` = Rt, `rm` = D register, `amt` = lane; `aux` bit 0: scalar -> core |
//! | `VMRS`/`VMSR` | `rd` = Rt (15 = APSR_nzcv for `VMRS`), `imm` = register (1 FPSCR, 0 FPSID, 5/6/7 MVFR2/1/0) |
//! | `VLDR`/`VSTR` | `rd`, `rn`, `imm` = signed byte offset (`rn` = 15: Align(PC, 4) based) |
//! | `VLDM`/`VSTM` | `rd` = first register, `rn`, `imm` = register count; `aux` bit 1 = write-back, bit 2 = decrement-before |
//! | `VPUSH`/`VPOP` | `rd` = first register, `imm` = register count |
//! | `VCVT_FI` (float -> integer) | `rd` = S destination, `rm` = source; `ra` = 1 signed; `s` = rounding `FR_*`; `aux` bit 0: source is double |
//! | `VCVT_IF` (integer -> float) | `rd` = destination, `rm` = S source; `ra` = 1 signed; `aux` bit 0: destination is double |
//! | `VCVT_FX` (fixed point) | `rd` = register; `imm` = fraction bits; `ra` = 1 signed; `aux` bit 0 double, bit 1 float -> fixed, bit 2 32-bit (else 16-bit) |
//! | `VCVT_DS` | `aux` bit 0: destination is double (`rd` = D, `rm` = S), else `rd` = S, `rm` = D |
//! | `VCVTB`/`VCVTT` | `aux` bit 0: -> half (else half -> float), bit 1: the other format is double |
//! | `VRINT` | `rd`, `rm`; `s` = mode `RI_*` |
//! | `VSEL` | `rd`, `rn`, `rm`; `s` = condition (0 EQ, 1 VS, 2 GE, 3 GT) |

use super::thumb::{ArmFeatures, Insn, Op};

/// `VCVT_FI` rounding selectors (`Insn::s`).
pub const FR_ZERO: u8 = 0;
pub const FR_FPSCR: u8 = 1;
pub const FR_AWAY: u8 = 2;
pub const FR_NEAREST: u8 = 3;
pub const FR_PLUS: u8 = 4;
pub const FR_MINUS: u8 = 5;

/// `VRINT` modes (`Insn::s`).
pub const RI_R: u8 = 0;
pub const RI_Z: u8 = 1;
pub const RI_X: u8 = 2;
pub const RI_A: u8 = 3;
pub const RI_N: u8 = 4;
pub const RI_P: u8 = 5;
pub const RI_M: u8 = 6;

/// FP system register selectors of `VMRS`/`VMSR` (`Insn::imm`).
pub const FPREG_FPSID: u32 = 0;
pub const FPREG_FPSCR: u32 = 1;
pub const FPREG_MVFR2: u32 = 5;
pub const FPREG_MVFR1: u32 = 6;
pub const FPREG_MVFR0: u32 = 7;

/// Single register index from a 4-bit field and the extra (low) bit.
#[inline]
fn sreg(v: u32, b: u32) -> u8 {
    (v << 1 | b) as u8
}

/// Double register index (FPv5-D16 has D0-D15 only: the extra bit must be 0).
#[inline]
fn dreg(v: u32, b: u32) -> Option<u8> {
    if b != 0 {
        None
    } else {
        Some(v as u8)
    }
}

/// Register of the selected precision.
#[inline]
fn reg(dp: bool, v: u32, b: u32) -> Option<u8> {
    if dp {
        dreg(v, b)
    } else {
        Some(sreg(v, b))
    }
}

/// VFPExpandImm: the top 32 bits of the double (or all 32 bits of the single) for `imm8`.
pub fn expand_imm8(imm8: u32, dp: bool) -> u32 {
    let sign = (imm8 >> 7) & 1;
    let b = (imm8 >> 6) & 1;
    let frac = imm8 & 0x3f; // imm8[5:0]
    if dp {
        // sign : NOT(b) : b x 8 : imm8[5:4] : imm8[3:0] : zeros(48)  -> top 16 bits then padding
        let exp = (!b & 1) << 10 | if b == 1 { 0xff << 2 } else { 0 } | (frac >> 4);
        sign << 31 | exp << 20 | (frac & 0xf) << 16
    } else {
        // sign : NOT(b) : b x 5 : imm8[5:4] : imm8[3:0] : zeros(19)
        let exp = (!b & 1) << 7 | if b == 1 { 0x1f << 2 } else { 0 } | (frac >> 4);
        sign << 31 | exp << 23 | (frac & 0xf) << 19
    }
}

/// Decodes the coprocessor space 0xEC00-0xEFFF (loads/stores, register transfers, data processing).
pub(super) fn decode_vfp(h1: u32, h2: u32, feat: ArmFeatures, i: &mut Insn) {
    if !feat.has_fpu() || (h2 >> 9) & 7 != 0b101 {
        return;
    }
    let dp = (h2 >> 8) & 1 == 1;
    if dp && !feat.has(ArmFeatures::FPV5_DP) {
        return;
    }
    match (h1 >> 8) & 0xf {
        0xc | 0xd => decode_ldst(h1, h2, dp, i),
        _ => {
            if h2 & 0x10 == 0 {
                decode_dp(h1, h2, dp, feat, i);
            } else {
                decode_transfer(h1, h2, dp, feat, i);
            }
        }
    }
}

fn decode_ldst(h1: u32, h2: u32, dp: bool, i: &mut Insn) {
    let rn = (h1 & 0xf) as u8;
    let p = (h1 >> 8) & 1;
    let u = (h1 >> 7) & 1;
    let d = (h1 >> 6) & 1;
    let w = (h1 >> 5) & 1;
    let l = (h1 >> 4) & 1;
    let vd = (h2 >> 12) & 0xf;
    let imm8 = h2 & 0xff;
    let dbit = dp as u8;
    if h1 & 0xffe0 == 0xec40 {
        // Two-register transfers: 1110 1100 010 L Rt2 | Rt 101x 00 M 1 Vm.
        if h2 & 0xd0 != 0x10 {
            return;
        }
        let rt = vd as u8;
        let m = (h2 >> 5) & 1;
        let vm = h2 & 0xf;
        let (op, rm) = if dp {
            let Some(r) = dreg(vm, m) else { return };
            (Op::VMOV_D2, r)
        } else {
            let r = sreg(vm, m);
            if r > 30 {
                return;
            }
            (Op::VMOV_2S, r)
        };
        if rt == 15 || rn == 15 || (l == 1 && rt == rn) {
            return;
        }
        i.op = op;
        i.rd = rt;
        i.rn = rn;
        i.rm = rm;
        i.aux = l as u8;
        return;
    }
    if p == 1 && w == 0 {
        // VLDR / VSTR.
        let Some(r) = reg(dp, vd, d) else { return };
        i.op = if l == 1 { Op::VLDR } else { Op::VSTR };
        i.rd = r;
        i.rn = rn;
        let off = imm8 << 2;
        i.imm = if u == 1 { off } else { off.wrapping_neg() };
        i.aux = dbit;
        return;
    }
    // Block transfers: P U W = 0 1 x (increment after), 1 0 1 (decrement before).
    let db = match (p, u, w) {
        (0, 1, _) => false,
        (1, 0, 1) => true,
        _ => return,
    };
    let (first, count) = if dp {
        if imm8 & 1 != 0 {
            return; // FLDMX / FSTMX
        }
        let Some(r) = dreg(vd, d) else { return };
        (r as u32, imm8 / 2)
    } else {
        (sreg(vd, d) as u32, imm8)
    };
    let limit = if dp { 16 } else { 32 };
    if count == 0 || first + count > limit || (w == 1 && rn == 15) {
        return;
    }
    i.rd = first as u8;
    i.rn = rn;
    i.imm = count;
    i.aux = dbit | (w as u8) << 1 | (db as u8) << 2;
    let sp_wb = rn == 13 && w == 1;
    i.op = match (l == 1, db) {
        (true, false) if sp_wb => Op::VPOP,
        (true, _) => Op::VLDM,
        (false, true) if sp_wb => Op::VPUSH,
        (false, _) => Op::VSTM,
    };
}

fn decode_transfer(h1: u32, h2: u32, dp: bool, feat: ArmFeatures, i: &mut Insn) {
    let rt = ((h2 >> 12) & 0xf) as u8;
    let l = (h1 >> 4) & 1;
    let n = (h2 >> 7) & 1;
    match (h1 >> 5) & 7 {
        // VMOV between a core register and a single-precision register.
        0b000 if !dp && h2 & 0x7f == 0x10 => {
            if rt == 15 {
                return;
            }
            i.op = if l == 1 { Op::VMOV_RS } else { Op::VMOV_SR };
            i.rd = rt;
            i.rm = sreg(h1 & 0xf, n);
        }
        // VMRS / VMSR.
        0b111 if !dp && h2 & 0xff == 0x10 => {
            let reg = h1 & 0xf;
            i.rd = rt;
            i.imm = reg;
            if l == 1 {
                if matches!(reg, 0 | 1 | 6 | 7) || (reg == 5 && feat.has(ArmFeatures::FPV5_DP)) {
                    i.op = Op::VMRS;
                }
            } else if reg == 1 {
                i.op = Op::VMSR;
            }
        }
        // VMOV (scalar): only the 32-bit form `vmov.32 Dd[x], Rt` exists for the VFP.
        0b001 | 0b000 if dp && h2 & 0x7f == 0x10 => {
            // opc1 = h1[6:5] = 0 x (x = lane), opc2 = h2[6:5] = 00.
            let Some(r) = dreg(h1 & 0xf, n) else { return };
            if rt == 15 {
                return;
            }
            i.op = Op::VMOV_SC;
            i.rd = rt;
            i.rm = r;
            i.amt = ((h1 >> 5) & 1) as u8;
            i.aux = l as u8;
        }
        _ => {}
    }
}

fn decode_dp(h1: u32, h2: u32, dp: bool, feat: ArmFeatures, i: &mut Insn) {
    if (h1 >> 8) & 0xf != 0xe {
        return;
    }
    let d = (h1 >> 6) & 1;
    let vn = h1 & 0xf;
    let vd = (h2 >> 12) & 0xf;
    let n = (h2 >> 7) & 1;
    let m = (h2 >> 5) & 1;
    let vm = h2 & 0xf;
    let s = (h2 >> 6) & 1;
    let p = (h1 >> 7) & 1;
    let q = (h1 >> 5) & 1;
    let r = (h1 >> 4) & 1;
    let dbit = dp as u8;
    if !(p == 1 && q == 1 && r == 1) {
        let (Some(rd), Some(rn), Some(rm)) = (reg(dp, vd, d), reg(dp, vn, n), reg(dp, vm, m)) else { return };
        let op = match (p, q, r, s) {
            (0, 0, 0, 0) => Op::VMLA,
            (0, 0, 0, 1) => Op::VMLS,
            (0, 0, 1, 0) => Op::VNMLS,
            (0, 0, 1, 1) => Op::VNMLA,
            (0, 1, 0, 0) => Op::VMUL,
            (0, 1, 0, 1) => Op::VNMUL,
            (0, 1, 1, 0) => Op::VADD,
            (0, 1, 1, 1) => Op::VSUB,
            (1, 0, 0, 0) => Op::VDIV,
            (1, 0, 1, 0) => Op::VFNMS,
            (1, 0, 1, 1) => Op::VFNMA,
            (1, 1, 0, 0) => Op::VFMA,
            (1, 1, 0, 1) => Op::VFMS,
            _ => return,
        };
        i.op = op;
        i.rd = rd;
        i.rn = rn;
        i.rm = rm;
        i.aux = dbit;
        return;
    }
    // Extension space: VMOV immediate and the unary / conversion operations.
    let opc2 = vn;
    let opc3 = (h2 >> 6) & 3;
    i.aux = dbit;
    if opc3 & 1 == 0 {
        if opc3 == 0 {
            let Some(rd) = reg(dp, vd, d) else { return };
            i.op = Op::VMOV_I;
            i.rd = rd;
            i.imm = expand_imm8(vn << 4 | vm, dp);
        }
        return;
    }
    let bit7 = (opc3 >> 1) & 1;
    let fv5 = feat.has(ArmFeatures::FPV5_DP);
    match opc2 {
        0b0000 | 0b0001 => {
            let (Some(rd), Some(rm)) = (reg(dp, vd, d), reg(dp, vm, m)) else { return };
            i.rd = rd;
            i.rm = rm;
            i.op = match (opc2, bit7) {
                (0, 0) => Op::VMOV_F,
                (0, _) => Op::VABS,
                (_, 0) => Op::VNEG,
                _ => Op::VSQRT,
            };
        }
        // VCVTB / VCVTT: half <-> single (and double with FPv5).
        0b0010 | 0b0011 => {
            if dp && !fv5 {
                return;
            }
            let to_half = opc2 & 1 == 1;
            i.op = if bit7 == 1 { Op::VCVTT } else { Op::VCVTB };
            i.aux = to_half as u8 | (dp as u8) << 1;
            if to_half {
                let (Some(rd), Some(rm)) = (Some(sreg(vd, d)), reg(dp, vm, m)) else { return };
                i.rd = rd;
                i.rm = rm;
            } else {
                let (Some(rd), rm) = (reg(dp, vd, d), sreg(vm, m)) else { return };
                i.rd = rd;
                i.rm = rm;
            }
        }
        // VCMP / VCMPE (register and with zero).
        0b0100 | 0b0101 => {
            let Some(rd) = reg(dp, vd, d) else { return };
            i.op = if bit7 == 1 { Op::VCMPE } else { Op::VCMP };
            i.rn = rd;
            if opc2 == 0b0101 {
                if vm != 0 || m != 0 {
                    i.op = Op::UNDEF;
                    return;
                }
                i.s = 1;
            } else {
                let Some(rm) = reg(dp, vm, m) else { return };
                i.rm = rm;
            }
        }
        // VRINTR / VRINTZ / VRINTX (FPv5).
        0b0110 | 0b0111 if opc2 == 0b0110 || bit7 == 0 => {
            if !fv5 {
                return;
            }
            let (Some(rd), Some(rm)) = (reg(dp, vd, d), reg(dp, vm, m)) else { return };
            i.op = Op::VRINT;
            i.rd = rd;
            i.rm = rm;
            i.s = match (opc2, bit7) {
                (0b0110, 0) => RI_R,
                (0b0110, _) => RI_Z,
                _ => RI_X,
            };
        }
        // VCVT between double and single precision.
        0b0111 => {
            if !fv5 {
                return;
            }
            i.op = Op::VCVT_DS;
            if dp {
                // f64 -> f32
                i.aux = 0;
                i.rd = sreg(vd, d);
                let Some(rm) = dreg(vm, m) else { return };
                i.rm = rm;
            } else {
                i.aux = 1;
                let Some(rd) = dreg(vd, d) else { return };
                i.rd = rd;
                i.rm = sreg(vm, m);
            }
        }
        // VCVT integer -> float.
        0b1000 => {
            let Some(rd) = reg(dp, vd, d) else { return };
            i.op = Op::VCVT_IF;
            i.rd = rd;
            i.rm = sreg(vm, m);
            i.ra = bit7 as u8;
        }
        // VCVT float <-> fixed point.
        0b1010 | 0b1011 | 0b1110 | 0b1111 => {
            let Some(rd) = reg(dp, vd, d) else { return };
            let size = if bit7 == 1 { 32 } else { 16 };
            let imm5 = vm << 1 | m;
            if imm5 > size {
                return;
            }
            i.op = Op::VCVT_FX;
            i.rd = rd;
            i.imm = size - imm5;
            i.ra = (opc2 & 1 == 0) as u8;
            i.aux = dbit | ((opc2 >> 2 & 1) as u8) << 1 | ((bit7 as u8) << 2);
        }
        // VCVT float -> integer.
        0b1100 | 0b1101 => {
            let Some(rm) = reg(dp, vm, m) else { return };
            i.op = Op::VCVT_FI;
            i.rd = sreg(vd, d);
            i.rm = rm;
            i.ra = (opc2 & 1) as u8;
            i.s = if bit7 == 1 { FR_ZERO } else { FR_FPSCR };
        }
        _ => {}
    }
}

/// FPv5 encodings under the 0xFE prefix: VSEL, VMAXNM/VMINNM, VRINTA/N/P/M, VCVTA/N/P/M.
pub(super) fn decode_vfp5(h1: u32, h2: u32, feat: ArmFeatures, i: &mut Insn) {
    if !feat.has(ArmFeatures::FPV5_DP) || (h1 >> 8) & 0xff != 0xfe || (h2 >> 9) & 7 != 0b101 {
        return;
    }
    let dp = (h2 >> 8) & 1 == 1;
    let d = (h1 >> 6) & 1;
    let vn = h1 & 0xf;
    let vd = (h2 >> 12) & 0xf;
    let n = (h2 >> 7) & 1;
    let m = (h2 >> 5) & 1;
    let vm = h2 & 0xf;
    i.aux = dp as u8;
    if h1 & 0x80 == 0 {
        // VSEL: 1111 1110 0 D cc Vn | Vd 101 sz N 0 M 0 Vm
        if h2 & 0x50 != 0 {
            return;
        }
        let (Some(rd), Some(rn), Some(rm)) = (reg(dp, vd, d), reg(dp, vn, n), reg(dp, vm, m)) else { return };
        i.op = Op::VSEL;
        i.rd = rd;
        i.rn = rn;
        i.rm = rm;
        i.s = ((h1 >> 4) & 3) as u8;
        return;
    }
    match (h1 >> 4) & 3 {
        0 => {
            // VMAXNM / VMINNM
            if h2 & 0x10 != 0 {
                return;
            }
            let (Some(rd), Some(rn), Some(rm)) = (reg(dp, vd, d), reg(dp, vn, n), reg(dp, vm, m)) else { return };
            i.op = if (h2 >> 6) & 1 == 1 { Op::VMINNM } else { Op::VMAXNM };
            i.rd = rd;
            i.rn = rn;
            i.rm = rm;
        }
        3 => {
            if (h2 >> 4) & 1 != 0 || (h2 >> 6) & 1 == 0 {
                return;
            }
            let rm_sel = (h1 & 3) as u8;
            if (h1 >> 2) & 3 == 2 {
                // VRINTA / N / P / M
                if (h2 >> 7) & 1 != 0 {
                    return;
                }
                let (Some(rd), Some(rm)) = (reg(dp, vd, d), reg(dp, vm, m)) else { return };
                i.op = Op::VRINT;
                i.rd = rd;
                i.rm = rm;
                i.s = RI_A + rm_sel;
            } else if (h1 >> 2) & 3 == 3 {
                // VCVTA / N / P / M (float -> integer)
                let Some(rm) = reg(dp, vm, m) else { return };
                i.op = Op::VCVT_FI;
                i.rd = sreg(vd, d);
                i.rm = rm;
                i.ra = ((h2 >> 7) & 1) as u8;
                i.s = FR_AWAY + rm_sel;
            }
        }
        _ => {}
    }
}
