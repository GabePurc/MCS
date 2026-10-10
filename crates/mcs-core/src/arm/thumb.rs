//! ARMv7-M Thumb / Thumb-2 instruction decoder.
//!
//! Sources: ARM DDI 0403E.e "ARMv7-M Architecture Reference Manual" chapter A5 (Thumb instruction
//! set encoding) and the instruction pages of chapter A7; Cortex-M4 DSP-extension encodings follow
//! ARMv7E-M (same manual, "E" suffixed pages).
//!
//! [`decode`] turns one instruction (one or two halfwords) into a compact, `Copy` [`Insn`]: an
//! [`Op`] id plus normalized operands. The executor (`mcs_sim::arm`) matches on `op`; the
//! disassembler ([`super::disasm`]) formats the very same `Insn`, so execution and listing can
//! never disagree about what an encoding means. Branch and literal offsets are stored relative to
//! the instruction's own address (`PC = address + 4`), so a decoded program is position independent
//! (flash is visible at 0x0000_0000 and at 0x0800_0000).
//!
//! # Operand layout
//!
//! | group | fields |
//! |---|---|
//! | data processing, immediate (`*_I`) | `rd`, `rn`, `imm`, `s`, `aux` = carry-out of the modified immediate (0/1) or 2 = leave C |
//! | data processing, register (`*_R`) | `rd`, `rn`, `rm`, `shift`, `amt`, `s` (`MOV_R` with a shift is LSL/LSR/ASR/ROR/RRX) |
//! | register-controlled shifts (`*_RV`) | `rd`, `rn` (value), `rm` (amount), `s` |
//! | loads / stores | `rd` (Rt), `ra` (Rt2 for LDRD/STRD), `rn`, `rm`, `imm` (signed offset), `amt` (LSL of `rm`), `aux` = addressing mode `AM_*` |
//! | LDM/STM/PUSH/POP | `rn`, `imm` = register list, `aux` = 1 for write-back |
//! | branches | `imm` = signed byte offset from `address + 4`; `B_COND` keeps the condition in `aux` |
//! | bit fields | `rd`, `rn`, `amt` = lsb, `aux` = width (`SSAT`/`USAT`: `imm` = saturation bit position) |
//! | DSP parallel add/sub (`PAR`) | `rd`, `rn`, `rm`, `aux` = prefix `PP_*`, `shift` = operation `PK_*` |
//! | DSP multiplies | `rd`, `rn`, `rm`, `ra`; `aux` = `x`/`y` halves (bit 0 = top half of `rn`, bit 1 = top half of `rm`), exchange (`X`) or rounding (`R`) flag; long forms: `rd` = RdLo, `ra` = RdHi |
//! | FP data processing | `rd`, `rn`, `rm` = S register (0-31) or D register (0-15) index, `aux` bit 0 = double precision |
//! | FP load/store | `rd` = first register, `rn` = base core register, `imm` = signed byte offset (`VLDR`/`VSTR`) or register count (`VLDM`...), `aux` bit 0 = double, bit 1 = write-back, bit 2 = decrement-before |
//! | FP moves/conversions | see the per-operation notes in `vfp.rs` |

#![allow(non_camel_case_types, clippy::upper_case_acronyms)]

use std::ops::BitOr;

/// Architecture feature mask handed to the decoder (extensions that are not part of base ARMv7-M).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ArmFeatures(pub u32);

impl ArmFeatures {
    /// ARMv7-M base profile (Cortex-M3).
    pub const BASE: ArmFeatures = ArmFeatures(0);
    /// ARMv7E-M DSP extension (saturating arithmetic, SIMD, extend-and-add, ...).
    pub const DSP: ArmFeatures = ArmFeatures(1);
    /// Single-precision FPU (FPv4-SP, Cortex-M4F).
    pub const FPV4_SP: ArmFeatures = ArmFeatures(2);
    /// Double-precision FPU (FPv5-D16, Cortex-M7).
    pub const FPV5_DP: ArmFeatures = ArmFeatures(4);
    /// Cortex-M4F.
    pub const CORTEX_M4F: ArmFeatures = ArmFeatures(1 | 2);
    /// Cortex-M7 with the double-precision FPU (FPv5-D16 includes the FPv4-SP instructions).
    pub const CORTEX_M7: ArmFeatures = ArmFeatures(1 | 2 | 4);

    /// Any FPU (single precision at least).
    #[inline]
    pub const fn has_fpu(self) -> bool {
        self.0 & 6 != 0
    }

    #[inline]
    pub const fn has(self, f: ArmFeatures) -> bool {
        self.0 & f.0 == f.0
    }
}

impl BitOr for ArmFeatures {
    type Output = ArmFeatures;
    fn bitor(self, rhs: ArmFeatures) -> ArmFeatures {
        ArmFeatures(self.0 | rhs.0)
    }
}

macro_rules! ops {
    ($($name:ident = $mn:expr,)*) => {
        /// Operation ids (dense, used by the executor's `match`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Op { $($name,)* }
        /// Canonical lower-case mnemonic of every operation, indexed by `Op as usize`.
        pub const OP_NAMES: &[&str] = &[$($mn,)*];
        /// Number of operations.
        pub const OP_COUNT: usize = OP_NAMES.len();
    };
}

ops! {
    UNDEF = "<undefined>", UDF = "udf",
    // Data processing, immediate operand.
    AND_I = "and", BIC_I = "bic", ORR_I = "orr", ORN_I = "orn", EOR_I = "eor",
    ADD_I = "add", ADC_I = "adc", SUB_I = "sub", SBC_I = "sbc", RSB_I = "rsb",
    MOV_I = "mov", MVN_I = "mvn",
    TST_I = "tst", TEQ_I = "teq", CMP_I = "cmp", CMN_I = "cmn",
    // Data processing, (shifted) register operand.
    AND_R = "and", BIC_R = "bic", ORR_R = "orr", ORN_R = "orn", EOR_R = "eor",
    ADD_R = "add", ADC_R = "adc", SUB_R = "sub", SBC_R = "sbc", RSB_R = "rsb",
    MOV_R = "mov", MVN_R = "mvn",
    TST_R = "tst", TEQ_R = "teq", CMP_R = "cmp", CMN_R = "cmn",
    // Register-controlled shifts.
    LSL_RV = "lsl", LSR_RV = "lsr", ASR_RV = "asr", ROR_RV = "ror",
    MOVT = "movt", ADR = "adr",
    // Multiply / divide.
    MUL = "mul", MLA = "mla", MLS = "mls",
    UMULL = "umull", SMULL = "smull", UMLAL = "umlal", SMLAL = "smlal",
    SDIV = "sdiv", UDIV = "udiv",
    // Bit manipulation, extension, saturation.
    BFC = "bfc", BFI = "bfi", UBFX = "ubfx", SBFX = "sbfx",
    CLZ = "clz", RBIT = "rbit", REV = "rev", REV16 = "rev16", REVSH = "revsh",
    SXTB = "sxtb", SXTH = "sxth", UXTB = "uxtb", UXTH = "uxth",
    SSAT = "ssat", USAT = "usat",
    QADD = "qadd", QSUB = "qsub", QDADD = "qdadd", QDSUB = "qdsub",
    // Loads / stores.
    LDR = "ldr", LDRB = "ldrb", LDRH = "ldrh", LDRSB = "ldrsb", LDRSH = "ldrsh",
    STR = "str", STRB = "strb", STRH = "strh",
    LDRT = "ldrt", LDRBT = "ldrbt", LDRHT = "ldrht", LDRSBT = "ldrsbt", LDRSHT = "ldrsht",
    STRT = "strt", STRBT = "strbt", STRHT = "strht",
    LDRD = "ldrd", STRD = "strd",
    LDM = "ldm", STM = "stm", LDMDB = "ldmdb", STMDB = "stmdb", PUSH = "push", POP = "pop",
    LDREX = "ldrex", LDREXB = "ldrexb", LDREXH = "ldrexh",
    STREX = "strex", STREXB = "strexb", STREXH = "strexh", CLREX = "clrex",
    PLD = "pld", PLI = "pli",
    // Branches.
    B = "b", B_COND = "b", BL = "bl", BX = "bx", BLX_R = "blx",
    CBZ = "cbz", CBNZ = "cbnz", TBB = "tbb", TBH = "tbh",
    // System.
    MRS = "mrs", MSR = "msr", CPS = "cps", SVC = "svc", BKPT = "bkpt",
    NOP = "nop", YIELD = "yield", WFE = "wfe", WFI = "wfi", SEV = "sev", DBG = "dbg",
    DMB = "dmb", DSB = "dsb", ISB = "isb", IT = "it",
    // DSP extension (ARMv7E-M): parallel add/subtract, packing, extension, dual / word / halfword
    // multiplies, sum of absolute differences.
    PAR = "parallel", SEL = "sel", USAD8 = "usad8", USADA8 = "usada8",
    SSAT16 = "ssat16", USAT16 = "usat16", PKH = "pkh", SXTB16 = "sxtb16", UXTB16 = "uxtb16",
    SMUL_XY = "smul", SMLA_XY = "smla", SMULW = "smulw", SMLAW = "smlaw", SMLAL_XY = "smlal",
    SMUAD = "smuad", SMUSD = "smusd", SMLAD = "smlad", SMLSD = "smlsd",
    SMLALD = "smlald", SMLSLD = "smlsld",
    SMMUL = "smmul", SMMLA = "smmla", SMMLS = "smmls", UMAAL = "umaal",
    // Floating point (FPv4-SP / FPv5-D16): loads, stores, moves.
    VLDR = "vldr", VSTR = "vstr", VLDM = "vldm", VSTM = "vstm", VPUSH = "vpush", VPOP = "vpop",
    VMOV_I = "vmov", VMOV_F = "vmov", VMOV_RS = "vmov", VMOV_SR = "vmov", VMOV_2S = "vmov",
    VMOV_D2 = "vmov", VMOV_SC = "vmov", VMRS = "vmrs", VMSR = "vmsr",
    // Floating point: arithmetic.
    VADD = "vadd", VSUB = "vsub", VMUL = "vmul", VNMUL = "vnmul", VDIV = "vdiv",
    VMLA = "vmla", VMLS = "vmls", VNMLA = "vnmla", VNMLS = "vnmls",
    VFMA = "vfma", VFMS = "vfms", VFNMA = "vfnma", VFNMS = "vfnms",
    VABS = "vabs", VNEG = "vneg", VSQRT = "vsqrt", VCMP = "vcmp", VCMPE = "vcmpe",
    // Floating point: conversions and the FPv5 additions.
    VCVT_FI = "vcvt", VCVT_IF = "vcvt", VCVT_FX = "vcvt", VCVT_DS = "vcvt", VCVTB = "vcvtb", VCVTT = "vcvtt",
    VRINT = "vrint", VSEL = "vsel", VMAXNM = "vmaxnm", VMINNM = "vminnm",
}

/// `PAR` prefixes (`Insn::aux`): signed, saturating, signed halving, unsigned, unsigned saturating,
/// unsigned halving.
pub const PP_S: u8 = 0;
pub const PP_Q: u8 = 1;
pub const PP_SH: u8 = 2;
pub const PP_U: u8 = 3;
pub const PP_UQ: u8 = 4;
pub const PP_UH: u8 = 5;
/// `PAR` operations (`Insn::shift`).
pub const PK_ADD16: u8 = 0;
pub const PK_ASX: u8 = 1;
pub const PK_SAX: u8 = 2;
pub const PK_SUB16: u8 = 3;
pub const PK_ADD8: u8 = 4;
pub const PK_SUB8: u8 = 5;

/// Flag-setting mode of `Insn::s`.
pub const S_NO: u8 = 0;
pub const S_YES: u8 = 1;
/// 16-bit encodings of data-processing instructions: flags are set only outside an IT block.
pub const S_NOT_IT: u8 = 2;

pub const SH_LSL: u8 = 0;
pub const SH_LSR: u8 = 1;
pub const SH_ASR: u8 = 2;
pub const SH_ROR: u8 = 3;
pub const SH_RRX: u8 = 4;

/// Load/store addressing modes (`Insn::aux`).
pub const AM_OFFSET: u8 = 0;
pub const AM_PRE: u8 = 1;
pub const AM_POST: u8 = 2;
/// `[rn, rm, LSL #amt]`
pub const AM_REG: u8 = 3;
/// PC-relative: address = Align(PC, 4) + imm (`rn` is 15).
pub const AM_LIT: u8 = 4;

/// A decoded instruction (16 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub op: Op,
    /// Encoding length in bytes: 2 or 4.
    pub len: u8,
    pub rd: u8,
    pub rn: u8,
    pub rm: u8,
    pub ra: u8,
    pub s: u8,
    pub shift: u8,
    pub amt: u8,
    pub aux: u8,
    pub imm: u32,
}

impl Insn {
    #[inline]
    pub const fn new(op: Op, len: u8) -> Insn {
        Insn { op, len, rd: 0, rn: 0, rm: 0, ra: 0, s: 0, shift: 0, amt: 0, aux: 0, imm: 0 }
    }

    /// Placeholder for flash words that have not been decoded.
    pub const UNDEF: Insn = Insn::new(Op::UNDEF, 2);
}

/// True when the first halfword starts a 32-bit encoding.
#[inline]
pub const fn is_32bit(hw1: u16) -> bool {
    (hw1 >> 11) >= 0b11101
}

#[inline]
fn sext(v: u32, bits: u32) -> u32 {
    (((v << (32 - bits)) as i32) >> (32 - bits)) as u32
}

/// DecodeImmShift: (shift kind, amount) for a 2-bit type and 5-bit immediate.
#[inline]
fn imm_shift(ty: u32, imm5: u32) -> (u8, u8) {
    match ty {
        0 => (SH_LSL, imm5 as u8),
        1 => (SH_LSR, if imm5 == 0 { 32 } else { imm5 as u8 }),
        2 => (SH_ASR, if imm5 == 0 { 32 } else { imm5 as u8 }),
        _ => {
            if imm5 == 0 {
                (SH_RRX, 1)
            } else {
                (SH_ROR, imm5 as u8)
            }
        }
    }
}

/// ThumbExpandImm_C: (value, carry-out as 0/1, or 2 when the carry flag is unchanged).
#[inline]
fn expand_imm(imm12: u32) -> (u32, u8) {
    if imm12 >> 10 == 0 {
        let b = imm12 & 0xff;
        let v = match (imm12 >> 8) & 3 {
            0 => b,
            1 => (b << 16) | b,
            2 => (b << 24) | (b << 8),
            _ => b * 0x0101_0101,
        };
        (v, 2)
    } else {
        let v = (0x80 | (imm12 & 0x7f)).rotate_right(imm12 >> 7);
        (v, (v >> 31) as u8)
    }
}

/// Decodes the instruction at the start of `(hw1, hw2)`; `hw2` is ignored for 16-bit encodings.
pub fn decode(hw1: u16, hw2: u16, feat: ArmFeatures) -> Insn {
    if is_32bit(hw1) {
        decode32(hw1 as u32, hw2 as u32, feat)
    } else {
        decode16(hw1 as u32)
    }
}

fn decode16(h: u32) -> Insn {
    let mut i = Insn::new(Op::UNDEF, 2);
    let r3 = |sh: u32| ((h >> sh) & 7) as u8;
    match h >> 12 {
        0b0000 | 0b0001 => {
            let op = (h >> 11) & 3;
            if op == 3 {
                let imm = (h >> 10) & 1 == 1;
                let sub = (h >> 9) & 1 == 1;
                i.op = match (imm, sub) {
                    (false, false) => Op::ADD_R,
                    (false, true) => Op::SUB_R,
                    (true, false) => Op::ADD_I,
                    (true, true) => Op::SUB_I,
                };
                i.rd = r3(0);
                i.rn = r3(3);
                if imm {
                    i.imm = (h >> 6) & 7;
                } else {
                    i.rm = r3(6);
                }
            } else {
                i.op = Op::MOV_R;
                i.rd = r3(0);
                i.rm = r3(3);
                let (k, a) = imm_shift(op, (h >> 6) & 0x1f);
                i.shift = k;
                i.amt = a;
            }
            i.s = S_NOT_IT;
        }
        0b0010 | 0b0011 => {
            let rd = r3(8);
            i.imm = h & 0xff;
            i.rd = rd;
            i.rn = rd;
            match (h >> 11) & 3 {
                0 => {
                    i.op = Op::MOV_I;
                    i.s = S_NOT_IT;
                    i.aux = 2;
                }
                1 => i.op = Op::CMP_I,
                2 => {
                    i.op = Op::ADD_I;
                    i.s = S_NOT_IT;
                    i.ra = 2;
                }
                _ => {
                    i.op = Op::SUB_I;
                    i.s = S_NOT_IT;
                    i.ra = 2;
                }
            }
        }
        0b0100 => {
            if h & 0x0800 != 0 {
                i.op = Op::LDR;
                i.rd = r3(8);
                i.rn = 15;
                i.aux = AM_LIT;
                i.imm = (h & 0xff) << 2;
            } else if h & 0x0400 == 0 {
                let rdn = r3(0);
                let rm = r3(3);
                i.rd = rdn;
                i.rn = rdn;
                i.rm = rm;
                i.s = S_NOT_IT;
                match (h >> 6) & 0xf {
                    0 => i.op = Op::AND_R,
                    1 => i.op = Op::EOR_R,
                    2 => i.op = Op::LSL_RV,
                    3 => i.op = Op::LSR_RV,
                    4 => i.op = Op::ASR_RV,
                    5 => i.op = Op::ADC_R,
                    6 => i.op = Op::SBC_R,
                    7 => i.op = Op::ROR_RV,
                    8 => {
                        i.op = Op::TST_R;
                        i.s = S_YES;
                    }
                    9 => {
                        i.op = Op::RSB_I;
                        i.rn = rm;
                        i.rm = 0;
                    }
                    10 => {
                        i.op = Op::CMP_R;
                        i.s = S_YES;
                    }
                    11 => {
                        i.op = Op::CMN_R;
                        i.s = S_YES;
                    }
                    12 => i.op = Op::ORR_R,
                    13 => {
                        i.op = Op::MUL;
                        i.rn = rm;
                        i.rm = rdn;
                    }
                    14 => i.op = Op::BIC_R,
                    _ => {
                        i.op = Op::MVN_R;
                        i.rn = 0;
                    }
                }
            } else {
                let rm = ((h >> 3) & 0xf) as u8;
                let dn = (((h >> 7) & 1) << 3 | (h & 7)) as u8;
                match (h >> 8) & 3 {
                    0 => {
                        i.op = Op::ADD_R;
                        i.rd = dn;
                        i.rn = dn;
                        i.rm = rm;
                        i.ra = 2;
                    }
                    1 => {
                        i.op = Op::CMP_R;
                        i.rn = dn;
                        i.rm = rm;
                        i.s = S_YES;
                    }
                    2 => {
                        i.op = Op::MOV_R;
                        i.rd = dn;
                        i.rm = rm;
                    }
                    _ => {
                        i.op = if h & 0x80 == 0 { Op::BX } else { Op::BLX_R };
                        i.rm = rm;
                    }
                }
            }
        }
        0b0101 => {
            i.op = [Op::STR, Op::STRH, Op::STRB, Op::LDRSB, Op::LDR, Op::LDRH, Op::LDRB, Op::LDRSH][((h >> 9) & 7) as usize];
            i.rd = r3(0);
            i.rn = r3(3);
            i.rm = r3(6);
            i.aux = AM_REG;
        }
        0b0110..=0b1000 => {
            let l = (h >> 11) & 1 == 1;
            let imm5 = (h >> 6) & 0x1f;
            let (op, scale) = match (h >> 12, l) {
                (6, false) => (Op::STR, 2),
                (6, true) => (Op::LDR, 2),
                (7, false) => (Op::STRB, 0),
                (7, true) => (Op::LDRB, 0),
                (_, false) => (Op::STRH, 1),
                (_, true) => (Op::LDRH, 1),
            };
            i.op = op;
            i.rd = r3(0);
            i.rn = r3(3);
            i.imm = imm5 << scale;
        }
        0b1001 => {
            i.op = if h & 0x0800 != 0 { Op::LDR } else { Op::STR };
            i.rd = r3(8);
            i.rn = 13;
            i.imm = (h & 0xff) << 2;
        }
        0b1010 => {
            i.rd = r3(8);
            i.imm = (h & 0xff) << 2;
            if h & 0x0800 == 0 {
                i.op = Op::ADR;
            } else {
                i.op = Op::ADD_I;
                i.rn = 13;
            }
        }
        0b1011 => decode16_misc(h, &mut i),
        0b1100 => {
            let l = h & 0x0800 != 0;
            i.op = if l { Op::LDM } else { Op::STM };
            i.rn = r3(8);
            i.imm = h & 0xff;
            // LDM writes back unless the base register is in the list; STM always.
            i.aux = (!l || i.imm & (1 << i.rn) == 0) as u8;
        }
        0b1101 => {
            let cond = (h >> 8) & 0xf;
            match cond {
                0xe => {
                    i.op = Op::UDF;
                    i.imm = h & 0xff;
                }
                0xf => {
                    i.op = Op::SVC;
                    i.imm = h & 0xff;
                }
                _ => {
                    i.op = Op::B_COND;
                    i.aux = cond as u8;
                    i.imm = sext((h & 0xff) << 1, 9);
                }
            }
        }
        0b1110 => {
            i.op = Op::B;
            i.imm = sext((h & 0x7ff) << 1, 12);
        }
        _ => {}
    }
    i
}

fn decode16_misc(h: u32, i: &mut Insn) {
    let r3 = |sh: u32| ((h >> sh) & 7) as u8;
    match (h >> 8) & 0xf {
        0b0000 => {
            i.op = if h & 0x80 == 0 { Op::ADD_I } else { Op::SUB_I };
            i.rd = 13;
            i.rn = 13;
            i.ra = 2;
            i.imm = (h & 0x7f) << 2;
        }
        0b0001 | 0b0011 | 0b1001 | 0b1011 => {
            i.op = if h & 0x0800 == 0 { Op::CBZ } else { Op::CBNZ };
            i.rn = r3(0);
            i.imm = ((h >> 9) & 1) << 6 | ((h >> 3) & 0x1f) << 1;
        }
        0b0010 => {
            i.op = [Op::SXTH, Op::SXTB, Op::UXTH, Op::UXTB][((h >> 6) & 3) as usize];
            i.rd = r3(0);
            i.rm = r3(3);
            i.rn = 15;
        }
        0b0100 | 0b0101 => {
            i.op = Op::PUSH;
            i.imm = (h & 0xff) | ((h >> 8) & 1) << 14;
        }
        0b0110 => {
            // 1011 0110 011 im 0 0 I F
            if (h >> 5) & 7 == 0b011 {
                i.op = Op::CPS;
                i.aux = ((h >> 4) & 1) as u8;
                i.imm = h & 3;
            }
        }
        0b1010 => match (h >> 6) & 3 {
            0 => {
                i.op = Op::REV;
                i.rd = r3(0);
                i.rm = r3(3);
            }
            1 => {
                i.op = Op::REV16;
                i.rd = r3(0);
                i.rm = r3(3);
            }
            3 => {
                i.op = Op::REVSH;
                i.rd = r3(0);
                i.rm = r3(3);
            }
            _ => {}
        },
        0b1100 | 0b1101 => {
            i.op = Op::POP;
            i.imm = (h & 0xff) | ((h >> 8) & 1) << 15;
        }
        0b1110 => {
            i.op = Op::BKPT;
            i.imm = h & 0xff;
        }
        0b1111 => {
            let mask = h & 0xf;
            if mask != 0 {
                i.op = Op::IT;
                i.imm = h & 0xff;
            } else {
                i.op = match (h >> 4) & 0xf {
                    1 => Op::YIELD,
                    2 => Op::WFE,
                    3 => Op::WFI,
                    4 => Op::SEV,
                    _ => Op::NOP,
                };
            }
        }
        _ => {}
    }
}

/// Maps the 4-bit data-processing opcode of the 32-bit encodings to its operations.
/// (immediate op, register op, compare ops used when `Rd == PC && S`).
type DpOps = (Op, Op, Option<(Op, Op)>);

fn dp_ops(op: u32) -> Option<DpOps> {
    Some(match op {
        0b0000 => (Op::AND_I, Op::AND_R, Some((Op::TST_I, Op::TST_R))),
        0b0001 => (Op::BIC_I, Op::BIC_R, None),
        0b0010 => (Op::ORR_I, Op::ORR_R, None),
        0b0011 => (Op::ORN_I, Op::ORN_R, None),
        0b0100 => (Op::EOR_I, Op::EOR_R, Some((Op::TEQ_I, Op::TEQ_R))),
        0b1000 => (Op::ADD_I, Op::ADD_R, Some((Op::CMN_I, Op::CMN_R))),
        0b1010 => (Op::ADC_I, Op::ADC_R, None),
        0b1011 => (Op::SBC_I, Op::SBC_R, None),
        0b1101 => (Op::SUB_I, Op::SUB_R, Some((Op::CMP_I, Op::CMP_R))),
        0b1110 => (Op::RSB_I, Op::RSB_R, None),
        _ => return None,
    })
}

fn decode32(h1: u32, h2: u32, feat: ArmFeatures) -> Insn {
    let mut i = Insn::new(Op::UNDEF, 4);
    match (h1 >> 11) & 3 {
        0b01 => decode32_01(h1, h2, feat, &mut i),
        0b10 => decode32_10(h1, h2, feat, &mut i),
        _ => decode32_11(h1, h2, feat, &mut i),
    }
    i
}

fn decode32_01(h1: u32, h2: u32, feat: ArmFeatures, i: &mut Insn) {
    let rn = (h1 & 0xf) as u8;
    match (h1 >> 9) & 3 {
        // Load/store multiple, dual/exclusive, table branch.
        0b00 => {
            if h1 & 0x40 == 0 {
                let l = h1 & 0x10 != 0;
                let w = (h1 >> 5) & 1;
                let list = h2 & 0xffff;
                i.rn = rn;
                i.imm = list;
                i.aux = w as u8;
                match (h1 >> 7) & 3 {
                    0b01 => {
                        if l && w == 1 && rn == 13 {
                            i.op = Op::POP;
                            i.aux = 0;
                        } else {
                            i.op = if l { Op::LDM } else { Op::STM };
                        }
                    }
                    0b10 => {
                        if !l && w == 1 && rn == 13 {
                            i.op = Op::PUSH;
                            i.aux = 0;
                        } else {
                            i.op = if l { Op::LDMDB } else { Op::STMDB };
                        }
                    }
                    _ => {}
                }
            } else {
                decode_dual_excl(h1, h2, i);
            }
        }
        // Data processing (shifted register).
        0b01 => {
            let op = (h1 >> 5) & 0xf;
            let s = (h1 >> 4) & 1;
            let rd = ((h2 >> 8) & 0xf) as u8;
            let rm = (h2 & 0xf) as u8;
            let (k, a) = imm_shift((h2 >> 4) & 3, ((h2 >> 12) & 7) << 2 | ((h2 >> 6) & 3));
            if op == 0b0110 {
                // PKHBT (type LSL) / PKHTB (type ASR) (DSP).
                if feat.has(ArmFeatures::DSP) && s == 0 && (h2 >> 4) & 1 == 0 {
                    i.op = Op::PKH;
                    i.rd = rd;
                    i.rn = rn;
                    i.rm = rm;
                    i.shift = k;
                    i.amt = a;
                }
                return;
            }
            let Some((_, rop, cmp)) = dp_ops(op) else { return };
            i.rd = rd;
            i.rn = rn;
            i.rm = rm;
            i.shift = k;
            i.amt = a;
            i.s = s as u8;
            if let (Some((_, cop)), true) = (cmp, rd == 15 && s == 1) {
                i.op = cop;
                i.rd = 0;
                i.s = S_YES;
            } else if op == 0b0010 && rn == 15 {
                i.op = Op::MOV_R;
                i.rn = 0;
            } else if op == 0b0011 && rn == 15 {
                i.op = Op::MVN_R;
                i.rn = 0;
            } else {
                i.op = rop;
            }
        }
        // Coprocessor space (0xEC-0xEF): FP loads/stores, moves and data processing.
        _ => super::vfp::decode_vfp(h1, h2, feat, i),
    }
}

fn decode_dual_excl(h1: u32, h2: u32, i: &mut Insn) {
    let rn = (h1 & 0xf) as u8;
    let op1 = (h1 >> 7) & 3;
    let op2 = (h1 >> 4) & 3;
    let rt = ((h2 >> 12) & 0xf) as u8;
    i.rn = rn;
    match (op1, op2) {
        (0, 0) | (0, 1) => {
            // STREX / LDREX
            i.imm = (h2 & 0xff) << 2;
            if op2 == 0 {
                i.op = Op::STREX;
                i.rd = ((h2 >> 8) & 0xf) as u8;
                i.rm = rt;
            } else {
                i.op = Op::LDREX;
                i.rd = rt;
            }
        }
        (1, 0) => {
            i.rd = (h2 & 0xf) as u8;
            i.rm = rt;
            match (h2 >> 4) & 0xf {
                4 => i.op = Op::STREXB,
                5 => i.op = Op::STREXH,
                _ => {}
            }
        }
        (1, 1) => match (h2 >> 4) & 0xf {
            0 => {
                i.op = Op::TBB;
                i.rm = (h2 & 0xf) as u8;
            }
            1 => {
                i.op = Op::TBH;
                i.rm = (h2 & 0xf) as u8;
            }
            4 => {
                i.op = Op::LDREXB;
                i.rd = rt;
            }
            5 => {
                i.op = Op::LDREXH;
                i.rd = rt;
            }
            _ => {}
        },
        _ => {
            // LDRD / STRD (immediate or literal): P = h1[8], U = h1[7], W = h1[5], L = h1[4].
            let p = (h1 >> 8) & 1;
            let u = (h1 >> 7) & 1;
            let w = (h1 >> 5) & 1;
            let l = (h1 >> 4) & 1;
            if p == 0 && w == 0 {
                return;
            }
            i.op = if l == 1 { Op::LDRD } else { Op::STRD };
            i.rd = rt;
            i.ra = ((h2 >> 8) & 0xf) as u8;
            let off = (h2 & 0xff) << 2;
            i.imm = if u == 1 { off } else { off.wrapping_neg() };
            i.aux = if rn == 15 {
                AM_LIT
            } else if p == 0 {
                AM_POST
            } else if w == 1 {
                AM_PRE
            } else {
                AM_OFFSET
            };
        }
    }
}

fn decode32_10(h1: u32, h2: u32, feat: ArmFeatures, i: &mut Insn) {
    let rn = (h1 & 0xf) as u8;
    let rd = ((h2 >> 8) & 0xf) as u8;
    if h2 & 0x8000 == 0 {
        let imm3 = (h2 >> 12) & 7;
        let imm8 = h2 & 0xff;
        let ibit = (h1 >> 10) & 1;
        if h1 & 0x0200 == 0 {
            // Modified immediate.
            let op = (h1 >> 5) & 0xf;
            let s = (h1 >> 4) & 1;
            let (v, c) = expand_imm(ibit << 11 | imm3 << 8 | imm8);
            let Some((iop, _, cmp)) = dp_ops(op) else { return };
            i.rd = rd;
            i.rn = rn;
            i.imm = v;
            i.aux = c;
            i.s = s as u8;
            if let (Some((cop, _)), true) = (cmp, rd == 15 && s == 1) {
                i.op = cop;
                i.rd = 0;
                i.s = S_YES;
            } else if op == 0b0010 && rn == 15 {
                i.op = Op::MOV_I;
                i.rn = 0;
            } else if op == 0b0011 && rn == 15 {
                i.op = Op::MVN_I;
                i.rn = 0;
            } else {
                i.op = iop;
            }
        } else {
            // Plain binary immediate.
            let op = (h1 >> 4) & 0x1f;
            let imm12 = ibit << 11 | imm3 << 8 | imm8;
            let imm5 = imm3 << 2 | ((h2 >> 6) & 3);
            i.rd = rd;
            i.rn = rn;
            match op {
                0b00000 | 0b01010 => {
                    if rn == 15 {
                        // ADR.W: ADD form adds, SUB form subtracts, both relative to Align(PC, 4).
                        i.op = Op::ADR;
                        i.imm = if op == 0 { imm12 } else { imm12.wrapping_neg() };
                    } else {
                        i.op = if op == 0 { Op::ADD_I } else { Op::SUB_I };
                        i.imm = imm12;
                        i.ra = 1;
                    }
                }
                0b00100 => {
                    i.op = Op::MOV_I;
                    i.rn = 0;
                    i.ra = 1;
                    i.imm = (h1 & 0xf) << 12 | imm12;
                    i.aux = 2;
                }
                0b01100 => {
                    i.op = Op::MOVT;
                    i.rn = 0;
                    i.imm = (h1 & 0xf) << 12 | imm12;
                }
                0b10000 | 0b10010 | 0b11000 | 0b11010 => {
                    let sh = (h1 >> 5) & 1;
                    if sh == 1 && imm5 == 0 {
                        // SSAT16 / USAT16 (DSP).
                        if feat.has(ArmFeatures::DSP) && h2 & 0x30 == 0 {
                            i.op = if op & 8 == 0 { Op::SSAT16 } else { Op::USAT16 };
                            i.imm = if op & 8 == 0 { (h2 & 0xf) + 1 } else { h2 & 0xf };
                        }
                        return;
                    }
                    i.op = if op & 8 == 0 { Op::SSAT } else { Op::USAT };
                    i.imm = if op & 8 == 0 { (h2 & 0x1f) + 1 } else { h2 & 0x1f };
                    i.shift = if sh == 1 { SH_ASR } else { SH_LSL };
                    i.amt = imm5 as u8;
                }
                0b10100 | 0b11100 => {
                    i.op = if op == 0b10100 { Op::SBFX } else { Op::UBFX };
                    i.amt = imm5 as u8;
                    i.aux = ((h2 & 0x1f) + 1) as u8;
                }
                0b10110 => {
                    let msb = h2 & 0x1f;
                    if msb < imm5 {
                        return;
                    }
                    i.op = if rn == 15 { Op::BFC } else { Op::BFI };
                    i.amt = imm5 as u8;
                    i.aux = (msb - imm5 + 1) as u8;
                }
                _ => {}
            }
        }
        return;
    }
    // Branches and miscellaneous control.
    match ((h2 >> 14) & 1, (h2 >> 12) & 1) {
        (0, 0) => {
            let hop = (h1 >> 4) & 0x7f;
            if hop == 0b1111111 {
                if (h2 >> 12) & 0xf == 0b1010 {
                    i.op = Op::UDF;
                    i.imm = ((h1 & 0xf) << 12) | (h2 & 0xfff);
                }
                return;
            }
            if hop & 0x38 != 0x38 {
                // B<cond>.W
                i.op = Op::B_COND;
                i.aux = ((h1 >> 6) & 0xf) as u8;
                let s = (h1 >> 10) & 1;
                let j1 = (h2 >> 13) & 1;
                let j2 = (h2 >> 11) & 1;
                let v = s << 20 | j2 << 19 | j1 << 18 | (h1 & 0x3f) << 12 | (h2 & 0x7ff) << 1;
                i.imm = sext(v, 21);
                return;
            }
            match hop {
                0b0111000 | 0b0111001 => {
                    // MSR
                    i.op = Op::MSR;
                    i.rn = rn;
                    i.imm = h2 & 0xff;
                    i.aux = ((h2 >> 10) & 3) as u8;
                }
                0b0111010 => {
                    if (h2 >> 8) & 7 == 0 && h2 & 0x600 == 0 {
                        let hint = h2 & 0xff;
                        i.op = match hint {
                            0 => Op::NOP,
                            1 => Op::YIELD,
                            2 => Op::WFE,
                            3 => Op::WFI,
                            4 => Op::SEV,
                            0xf0..=0xff => {
                                i.imm = hint & 0xf;
                                Op::DBG
                            }
                            _ => Op::NOP,
                        };
                    } else if (h2 >> 9) & 3 >= 2 && h1 == 0xf3af {
                        i.op = Op::CPS;
                        i.aux = ((h2 >> 9) & 1) as u8;
                        i.imm = ((h2 >> 6) & 1) << 1 | ((h2 >> 5) & 1);
                    }
                }
                0b0111011 => match (h2 >> 4) & 0xf {
                    2 => i.op = Op::CLREX,
                    4 => {
                        i.op = Op::DSB;
                        i.imm = h2 & 0xf;
                    }
                    5 => {
                        i.op = Op::DMB;
                        i.imm = h2 & 0xf;
                    }
                    6 => {
                        i.op = Op::ISB;
                        i.imm = h2 & 0xf;
                    }
                    _ => {}
                },
                0b0111110 | 0b0111111 => {
                    i.op = Op::MRS;
                    i.rd = rd;
                    i.imm = h2 & 0xff;
                }
                _ => {}
            }
        }
        (_, 1) => {
            // B.W (T4) / BL
            let s = (h1 >> 10) & 1;
            let j1 = (h2 >> 13) & 1;
            let j2 = (h2 >> 11) & 1;
            let i1 = !(j1 ^ s) & 1;
            let i2 = !(j2 ^ s) & 1;
            let v = s << 24 | i1 << 23 | i2 << 22 | (h1 & 0x3ff) << 12 | (h2 & 0x7ff) << 1;
            i.imm = sext(v, 25);
            i.op = if (h2 >> 14) & 1 == 1 { Op::BL } else { Op::B };
        }
        _ => {
            let _ = feat;
        }
    }
}

fn decode32_11(h1: u32, h2: u32, feat: ArmFeatures, i: &mut Insn) {
    let rn = (h1 & 0xf) as u8;
    match (h1 >> 8) & 0xf {
        // Load/store single data item (1111 100x).
        0b1000 | 0b1001 => decode_ldst_single(h1, h2, i),
        // FPv5 additions (VSEL, VMAXNM, VRINT*, VCVTA/N/P/M).
        0b1110 => super::vfp::decode_vfp5(h1, h2, feat, i),
        // Data processing (register): 1111 1010.
        0b1010 => {
            let op1 = (h1 >> 4) & 0xf;
            let op2 = (h2 >> 4) & 0xf;
            let rd = ((h2 >> 8) & 0xf) as u8;
            let rm = (h2 & 0xf) as u8;
            if h2 & 0xf000 != 0xf000 {
                return;
            }
            i.rd = rd;
            i.rn = rn;
            i.rm = rm;
            if op1 & 8 == 0 {
                if op2 == 0 {
                    i.op = [Op::LSL_RV, Op::LSR_RV, Op::ASR_RV, Op::ROR_RV][((op1 >> 1) & 3) as usize];
                    i.s = (op1 & 1) as u8;
                } else if op2 & 8 != 0 {
                    // Extend (and add): SXTH/UXTH/SXTB/UXTB; the 16-bit-pair forms are DSP only.
                    let op = match op1 {
                        0 => Op::SXTH,
                        1 => Op::UXTH,
                        2 => Op::SXTB16,
                        3 => Op::UXTB16,
                        4 => Op::SXTB,
                        5 => Op::UXTB,
                        _ => return,
                    };
                    if (rn != 15 || matches!(op, Op::SXTB16 | Op::UXTB16)) && !feat.has(ArmFeatures::DSP) {
                        return;
                    }
                    i.op = op;
                    i.amt = (((h2 >> 4) & 3) * 8) as u8;
                }
            } else if (h2 >> 7) & 1 == 0 {
                // Parallel addition and subtraction (DSP): op1 = 1 kind(3), prefix in h2[6:4].
                let kind = match op1 & 7 {
                    0 => PK_ADD8,
                    1 => PK_ADD16,
                    2 => PK_ASX,
                    4 => PK_SUB8,
                    5 => PK_SUB16,
                    6 => PK_SAX,
                    _ => return,
                };
                let pfx = (h2 >> 4) & 3;
                if pfx == 3 || !feat.has(ArmFeatures::DSP) {
                    return;
                }
                i.op = Op::PAR;
                i.shift = kind;
                i.aux = pfx as u8 + 3 * ((h2 >> 6) & 1) as u8;
            } else if op1 & 0xc == 0x8 && (h2 >> 6) & 3 == 2 {
                // Miscellaneous operations.
                let sel = ((h1 >> 4) & 3, (h2 >> 4) & 3);
                let (op, dsp) = match sel {
                    (0, 0) => (Op::QADD, true),
                    (0, 1) => (Op::QDADD, true),
                    (0, 2) => (Op::QSUB, true),
                    (0, 3) => (Op::QDSUB, true),
                    (2, 0) => (Op::SEL, true),
                    (1, 0) => (Op::REV, false),
                    (1, 1) => (Op::REV16, false),
                    (1, 2) => (Op::RBIT, false),
                    (1, 3) => (Op::REVSH, false),
                    (3, 0) => (Op::CLZ, false),
                    _ => return,
                };
                if dsp && !feat.has(ArmFeatures::DSP) {
                    return;
                }
                i.op = op;
                if !dsp {
                    // REV/RBIT/CLZ repeat Rm in the Rn field.
                    i.rn = rm;
                }
            }
        }
        // Multiply (1111 1011 0) and long multiply / divide (1111 1011 1).
        0b1011 => {
            let ra = ((h2 >> 12) & 0xf) as u8;
            let rd = ((h2 >> 8) & 0xf) as u8;
            let rm = (h2 & 0xf) as u8;
            let dsp = feat.has(ArmFeatures::DSP);
            i.rn = rn;
            i.rm = rm;
            if h1 & 0x80 == 0 {
                if (h2 >> 6) & 3 != 0 {
                    return;
                }
                let op1 = (h1 >> 4) & 7;
                let op2 = (h2 >> 4) & 3;
                i.rd = rd;
                i.ra = ra;
                let mul_only = ra == 15;
                // Bit 0 of `aux`: top half of Rn (N), bit 1: top half of Rm (M).
                let nm = ((op2 >> 1) | ((op2 & 1) << 1)) as u8;
                match op1 {
                    0 => match op2 {
                        0 => i.op = if mul_only { Op::MUL } else { Op::MLA },
                        1 => i.op = Op::MLS,
                        _ => {}
                    },
                    _ if !dsp => {}
                    1 => {
                        i.op = if mul_only { Op::SMUL_XY } else { Op::SMLA_XY };
                        i.aux = nm;
                    }
                    2..=6 if op2 > 1 => {}
                    2 => {
                        i.op = if mul_only { Op::SMUAD } else { Op::SMLAD };
                        i.aux = op2 as u8;
                    }
                    3 => {
                        i.op = if mul_only { Op::SMULW } else { Op::SMLAW };
                        i.aux = (op2 as u8) << 1;
                    }
                    4 => {
                        i.op = if mul_only { Op::SMUSD } else { Op::SMLSD };
                        i.aux = op2 as u8;
                    }
                    5 => {
                        i.op = if mul_only { Op::SMMUL } else { Op::SMMLA };
                        i.aux = op2 as u8;
                    }
                    6 => {
                        i.op = Op::SMMLS;
                        i.aux = op2 as u8;
                    }
                    _ => {
                        if op2 == 0 {
                            i.op = if mul_only { Op::USAD8 } else { Op::USADA8 };
                        }
                    }
                }
            } else {
                let op1 = (h1 >> 4) & 7;
                let op2 = (h2 >> 4) & 0xf;
                match (op1, op2) {
                    (0, 0) => i.op = Op::SMULL,
                    (2, 0) => i.op = Op::UMULL,
                    (4, 0) => i.op = Op::SMLAL,
                    (6, 0) => i.op = Op::UMLAL,
                    (1, 0xf) if ra == 0xf => {
                        i.op = Op::SDIV;
                        i.rd = rd;
                        return;
                    }
                    (3, 0xf) if ra == 0xf => {
                        i.op = Op::UDIV;
                        i.rd = rd;
                        return;
                    }
                    _ if !dsp => return,
                    (4, 8..=0xb) => {
                        i.op = Op::SMLAL_XY;
                        i.aux = (((op2 >> 1) & 1) | ((op2 & 1) << 1)) as u8;
                    }
                    (4, 0xc | 0xd) => {
                        i.op = Op::SMLALD;
                        i.aux = (op2 & 1) as u8;
                    }
                    (5, 0xc | 0xd) => {
                        i.op = Op::SMLSLD;
                        i.aux = (op2 & 1) as u8;
                    }
                    (6, 6) => i.op = Op::UMAAL,
                    _ => return,
                }
                // RdLo = Rt field (h2[15:12]), RdHi = h2[11:8].
                i.rd = ra;
                i.ra = rd;
            }
        }
        _ => {}
    }
}

fn decode_ldst_single(h1: u32, h2: u32, i: &mut Insn) {
    // h1 = 1111 1000 S I12 sz L Rn  (bit 8 = S, bit 7 = imm12 flag / U for literals).
    let rn = (h1 & 0xf) as u8;
    let l = h1 & 0x10 != 0;
    let sz = (h1 >> 5) & 3;
    let sign = h1 & 0x100 != 0;
    let big = h1 & 0x80 != 0;
    let rt = ((h2 >> 12) & 0xf) as u8;
    if h1 & 0x600 != 0 {
        return;
    }
    // Operation for (load, sign, size) with the plain / unprivileged variants.
    let base = match (l, sign, sz) {
        (false, false, 0) => Op::STRB,
        (false, false, 1) => Op::STRH,
        (false, false, 2) => Op::STR,
        (true, false, 0) => Op::LDRB,
        (true, true, 0) => Op::LDRSB,
        (true, false, 1) => Op::LDRH,
        (true, true, 1) => Op::LDRSH,
        (true, false, 2) => Op::LDR,
        _ => return,
    };
    i.rd = rt;
    i.rn = rn;
    if l && rn == 15 {
        // Literal: U = h1[7].
        i.aux = AM_LIT;
        let off = h2 & 0xfff;
        i.imm = if big { off } else { off.wrapping_neg() };
        i.op = base;
    } else if big {
        i.imm = h2 & 0xfff;
        i.op = base;
    } else if h2 & 0x800 == 0 {
        if h2 >> 6 & 0x1f != 0 {
            return;
        }
        i.aux = AM_REG;
        i.rm = (h2 & 0xf) as u8;
        i.amt = ((h2 >> 4) & 3) as u8;
        i.op = base;
    } else {
        let p = (h2 >> 10) & 1;
        let u = (h2 >> 9) & 1;
        let w = (h2 >> 8) & 1;
        let imm8 = h2 & 0xff;
        i.op = base;
        if p == 1 && u == 1 && w == 0 {
            // Unprivileged access.
            i.op = match base {
                Op::LDR => Op::LDRT,
                Op::LDRB => Op::LDRBT,
                Op::LDRH => Op::LDRHT,
                Op::LDRSB => Op::LDRSBT,
                Op::LDRSH => Op::LDRSHT,
                Op::STR => Op::STRT,
                Op::STRB => Op::STRBT,
                _ => Op::STRHT,
            };
            i.imm = imm8;
        } else if p == 0 && w == 0 {
            // Undefined (store) / PLD-like space (load): not a real access.
            i.op = Op::UNDEF;
            return;
        } else {
            i.imm = if u == 1 { imm8 } else { imm8.wrapping_neg() };
            i.aux = if p == 0 {
                AM_POST
            } else if w == 1 {
                AM_PRE
            } else {
                AM_OFFSET
            };
        }
    }
    // Loads into PC-field 15 of byte/halfword size are memory hints.
    if l && rt == 15 {
        match base {
            Op::LDRB => i.op = Op::PLD,
            Op::LDRH => i.op = Op::NOP,
            Op::LDRSB => i.op = Op::PLI,
            Op::LDRSH => i.op = Op::NOP,
            _ => {}
        }
        if matches!(i.op, Op::PLD | Op::PLI) {
            i.ra = 0;
        }
    }
}
