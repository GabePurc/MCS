//! RV32IMC + Zicsr + Zifencei decoder into a compact pre-decoded form.
//!
//! [`decode`] turns the first 16 or 32 bits of an instruction into an [`Insn`]: compressed (RVC)
//! instructions are expanded to their base form (the 16-bit length is kept in [`Insn::len`]), so
//! the executor only knows the base operations of [`Op`]. Anything that is not RV32IMC +
//! Zicsr/Zifencei + the machine-mode instructions `mret`/`wfi` decodes to [`Op::Illegal`] (FP and
//! atomic encodings, the RV64-only compressed forms, reserved encodings, `sret`, `sfence.vma`...).
//!
//! References: The RISC-V Instruction Set Manual Volume I (Unprivileged, ch. 2, 7, 12, 16, 24),
//! Volume II (Privileged, `mret`/`wfi`/CSR instructions).

/// Base operation of a decoded instruction.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// Pre-decode cache sentinel: this slot has not been decoded yet. Never returned by [`decode`].
    Undecoded,
    /// Undefined / unsupported encoding; `imm` holds the instruction bits (for `mtval`).
    Illegal,
    Lui,
    Auipc,
    Jal,
    Jalr,
    Beq,
    Bne,
    Blt,
    Bge,
    Bltu,
    Bgeu,
    Lb,
    Lh,
    Lw,
    Lbu,
    Lhu,
    Sb,
    Sh,
    Sw,
    Addi,
    Slti,
    Sltiu,
    Xori,
    Ori,
    Andi,
    Slli,
    Srli,
    Srai,
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
    /// `fence`; `imm` = instruction bits 31:20 (fm / predecessor / successor).
    Fence,
    FenceI,
    Ecall,
    Ebreak,
    Mret,
    Wfi,
    /// CSR instructions: `imm` = CSR number, `rs1` = source register (`Csrrw`/`Csrrs`/`Csrrc`) or
    /// the 5-bit immediate (`Csrrwi`/`Csrrsi`/`Csrrci`).
    Csrrw,
    Csrrs,
    Csrrc,
    Csrrwi,
    Csrrsi,
    Csrrci,
}

/// A pre-decoded instruction.
///
/// Operand use per [`Op`]: ALU register ops `rd, rs1, rs2`; immediate ops `rd, rs1, imm`; loads
/// `rd, imm(rs1)`; stores `rs2, imm(rs1)`; branches compare `rs1, rs2` and jump by `imm`; `Jal`
/// `rd, imm`; `Jalr` `rd, imm(rs1)`; `Lui`/`Auipc` `imm` already shifted left by 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub imm: i32,
    pub op: Op,
    pub rd: u8,
    pub rs1: u8,
    pub rs2: u8,
    /// Encoded length in bytes: 2 (RVC) or 4.
    pub len: u8,
}

impl Insn {
    /// Pre-decode cache sentinel (see [`Op::Undecoded`]).
    pub const UNDECODED: Insn = Insn { imm: 0, op: Op::Undecoded, rd: 0, rs1: 0, rs2: 0, len: 2 };
}

/// Length in bytes (2 or 4) of the instruction whose first halfword is `half`. Encodings longer
/// than 32 bits (low five bits all ones) are reported as 4 and decode as illegal.
#[inline]
pub fn insn_len(half: u16) -> u8 {
    if half & 3 == 3 {
        4
    } else {
        2
    }
}

#[inline]
fn bits(w: u32, hi: u32, lo: u32) -> u32 {
    (w >> lo) & ((1u32 << (hi - lo + 1)) - 1)
}

#[inline]
fn sext(v: u32, nbits: u32) -> i32 {
    ((v << (32 - nbits)) as i32) >> (32 - nbits)
}

#[inline]
const fn mk(op: Op, rd: u32, rs1: u32, rs2: u32, imm: i32, len: u8) -> Insn {
    Insn { imm, op, rd: rd as u8, rs1: rs1 as u8, rs2: rs2 as u8, len }
}

fn illegal(w: u32, len: u8) -> Insn {
    let bitsv = if len == 2 { w & 0xffff } else { w };
    mk(Op::Illegal, 0, 0, 0, bitsv as i32, len)
}

/// Decodes the instruction at the start of `w`: the low 16 bits are the first halfword; for a
/// 32-bit instruction (low two bits `11`) the upper 16 bits hold the second halfword (ignored for
/// 16-bit instructions).
pub fn decode(w: u32) -> Insn {
    if w & 3 == 3 {
        decode32(w)
    } else {
        decode16(w & 0xffff)
    }
}

fn decode32(w: u32) -> Insn {
    let rd = bits(w, 11, 7);
    let rs1 = bits(w, 19, 15);
    let rs2 = bits(w, 24, 20);
    let f3 = bits(w, 14, 12);
    let f7 = bits(w, 31, 25);
    let imm_i = (w as i32) >> 20;
    let bad = || illegal(w, 4);
    match bits(w, 6, 2) {
        0x0D => mk(Op::Lui, rd, 0, 0, (w & 0xffff_f000) as i32, 4),
        0x05 => mk(Op::Auipc, rd, 0, 0, (w & 0xffff_f000) as i32, 4),
        0x1B => {
            let imm = (bits(w, 31, 31) << 20) | (bits(w, 19, 12) << 12) | (bits(w, 20, 20) << 11) | (bits(w, 30, 21) << 1);
            mk(Op::Jal, rd, 0, 0, sext(imm, 21), 4)
        }
        0x19 if f3 == 0 => mk(Op::Jalr, rd, rs1, 0, imm_i, 4),
        0x18 => {
            let imm = (bits(w, 31, 31) << 12) | (bits(w, 7, 7) << 11) | (bits(w, 30, 25) << 5) | (bits(w, 11, 8) << 1);
            let op = match f3 {
                0 => Op::Beq,
                1 => Op::Bne,
                4 => Op::Blt,
                5 => Op::Bge,
                6 => Op::Bltu,
                7 => Op::Bgeu,
                _ => return bad(),
            };
            mk(op, 0, rs1, rs2, sext(imm, 13), 4)
        }
        0x00 => {
            let op = match f3 {
                0 => Op::Lb,
                1 => Op::Lh,
                2 => Op::Lw,
                4 => Op::Lbu,
                5 => Op::Lhu,
                _ => return bad(),
            };
            mk(op, rd, rs1, 0, imm_i, 4)
        }
        0x08 => {
            let op = match f3 {
                0 => Op::Sb,
                1 => Op::Sh,
                2 => Op::Sw,
                _ => return bad(),
            };
            mk(op, 0, rs1, rs2, sext((f7 << 5) | rd, 12), 4)
        }
        0x04 => {
            let op = match f3 {
                0 => Op::Addi,
                2 => Op::Slti,
                3 => Op::Sltiu,
                4 => Op::Xori,
                6 => Op::Ori,
                7 => Op::Andi,
                1 if f7 == 0 => return mk(Op::Slli, rd, rs1, 0, rs2 as i32, 4),
                5 if f7 == 0 => return mk(Op::Srli, rd, rs1, 0, rs2 as i32, 4),
                5 if f7 == 0x20 => return mk(Op::Srai, rd, rs1, 0, rs2 as i32, 4),
                _ => return bad(),
            };
            mk(op, rd, rs1, 0, imm_i, 4)
        }
        0x0C => {
            let op = match (f7, f3) {
                (0, 0) => Op::Add,
                (0x20, 0) => Op::Sub,
                (0, 1) => Op::Sll,
                (0, 2) => Op::Slt,
                (0, 3) => Op::Sltu,
                (0, 4) => Op::Xor,
                (0, 5) => Op::Srl,
                (0x20, 5) => Op::Sra,
                (0, 6) => Op::Or,
                (0, 7) => Op::And,
                (1, 0) => Op::Mul,
                (1, 1) => Op::Mulh,
                (1, 2) => Op::Mulhsu,
                (1, 3) => Op::Mulhu,
                (1, 4) => Op::Div,
                (1, 5) => Op::Divu,
                (1, 6) => Op::Rem,
                (1, 7) => Op::Remu,
                _ => return bad(),
            };
            mk(op, rd, rs1, rs2, 0, 4)
        }
        0x03 => match f3 {
            // fm = 0 (ordinary fence) or fm = 8 with rw,rw (fence.tso); other fm values are reserved.
            0 if rd == 0 && rs1 == 0 && (w >> 28 == 0 || w >> 20 == 0x833) => mk(Op::Fence, 0, 0, 0, (w >> 20) as i32, 4),
            1 if w == 0x0000_100f => mk(Op::FenceI, 0, 0, 0, 0, 4),
            _ => bad(),
        },
        0x1C => match f3 {
            0 => match w {
                0x0000_0073 => mk(Op::Ecall, 0, 0, 0, 0, 4),
                0x0010_0073 => mk(Op::Ebreak, 0, 0, 0, 0, 4),
                0x3020_0073 => mk(Op::Mret, 0, 0, 0, 0, 4),
                0x1050_0073 => mk(Op::Wfi, 0, 0, 0, 0, 4),
                _ => bad(),
            },
            4 => bad(),
            _ => {
                let op = match f3 {
                    1 => Op::Csrrw,
                    2 => Op::Csrrs,
                    3 => Op::Csrrc,
                    5 => Op::Csrrwi,
                    6 => Op::Csrrsi,
                    _ => Op::Csrrci,
                };
                mk(op, rd, rs1, 0, (w >> 20) as i32, 4)
            }
        },
        _ => bad(),
    }
}

fn decode16(h: u32) -> Insn {
    let bad = || illegal(h, 2);
    let f3 = bits(h, 15, 13);
    let rd_full = bits(h, 11, 7);
    let rs2_full = bits(h, 6, 2);
    let rdp = 8 + bits(h, 4, 2);
    let rs1p = 8 + bits(h, 9, 7);
    match h & 3 {
        0 => match f3 {
            0 => {
                // c.addi4spn
                let nz = (bits(h, 12, 11) << 4) | (bits(h, 10, 7) << 6) | (bits(h, 6, 6) << 2) | (bits(h, 5, 5) << 3);
                if nz == 0 {
                    return bad();
                }
                mk(Op::Addi, rdp, 2, 0, nz as i32, 2)
            }
            2 | 6 => {
                let off = (bits(h, 12, 10) << 3) | (bits(h, 6, 6) << 2) | (bits(h, 5, 5) << 6);
                if f3 == 2 {
                    mk(Op::Lw, rdp, rs1p, 0, off as i32, 2)
                } else {
                    mk(Op::Sw, 0, rs1p, rdp, off as i32, 2)
                }
            }
            _ => bad(),
        },
        1 => match f3 {
            0 => mk(Op::Addi, rd_full, rd_full, 0, sext((bits(h, 12, 12) << 5) | rs2_full, 6), 2),
            1 | 5 => {
                let off = (bits(h, 12, 12) << 11)
                    | (bits(h, 11, 11) << 4)
                    | (bits(h, 10, 9) << 8)
                    | (bits(h, 8, 8) << 10)
                    | (bits(h, 7, 7) << 6)
                    | (bits(h, 6, 6) << 7)
                    | (bits(h, 5, 3) << 1)
                    | (bits(h, 2, 2) << 5);
                mk(Op::Jal, if f3 == 1 { 1 } else { 0 }, 0, 0, sext(off, 12), 2)
            }
            2 => mk(Op::Addi, rd_full, 0, 0, sext((bits(h, 12, 12) << 5) | rs2_full, 6), 2),
            3 => {
                if rd_full == 2 {
                    let nz = (bits(h, 12, 12) << 9) | (bits(h, 6, 6) << 4) | (bits(h, 5, 5) << 6) | (bits(h, 4, 3) << 7) | (bits(h, 2, 2) << 5);
                    if nz == 0 {
                        return bad();
                    }
                    mk(Op::Addi, 2, 2, 0, sext(nz, 10), 2)
                } else {
                    let nz = (bits(h, 12, 12) << 17) | (bits(h, 6, 2) << 12);
                    if nz == 0 {
                        return bad();
                    }
                    mk(Op::Lui, rd_full, 0, 0, sext(nz, 18), 2)
                }
            }
            4 => {
                let shamt = (bits(h, 12, 12) << 5) | rs2_full;
                match bits(h, 11, 10) {
                    0 | 1 => {
                        if shamt >= 32 {
                            return bad();
                        }
                        let op = if bits(h, 11, 10) == 0 { Op::Srli } else { Op::Srai };
                        mk(op, rs1p, rs1p, 0, shamt as i32, 2)
                    }
                    2 => mk(Op::Andi, rs1p, rs1p, 0, sext(shamt, 6), 2),
                    _ => {
                        if bits(h, 12, 12) != 0 {
                            return bad();
                        }
                        let op = match bits(h, 6, 5) {
                            0 => Op::Sub,
                            1 => Op::Xor,
                            2 => Op::Or,
                            _ => Op::And,
                        };
                        mk(op, rs1p, rs1p, rdp, 0, 2)
                    }
                }
            }
            _ => {
                // c.beqz / c.bnez
                let off = (bits(h, 12, 12) << 8)
                    | (bits(h, 11, 10) << 3)
                    | (bits(h, 6, 5) << 6)
                    | (bits(h, 4, 3) << 1)
                    | (bits(h, 2, 2) << 5);
                mk(if f3 == 6 { Op::Beq } else { Op::Bne }, 0, rs1p, 0, sext(off, 9), 2)
            }
        },
        _ => match f3 {
            0 => {
                let shamt = (bits(h, 12, 12) << 5) | rs2_full;
                if shamt >= 32 {
                    return bad();
                }
                mk(Op::Slli, rd_full, rd_full, 0, shamt as i32, 2)
            }
            2 => {
                if rd_full == 0 {
                    return bad();
                }
                let off = (bits(h, 12, 12) << 5) | (bits(h, 6, 4) << 2) | (bits(h, 3, 2) << 6);
                mk(Op::Lw, rd_full, 2, 0, off as i32, 2)
            }
            4 => {
                if bits(h, 12, 12) == 0 {
                    if rs2_full == 0 {
                        if rd_full == 0 {
                            return bad();
                        }
                        mk(Op::Jalr, 0, rd_full, 0, 0, 2)
                    } else {
                        mk(Op::Add, rd_full, 0, rs2_full, 0, 2)
                    }
                } else if rs2_full == 0 {
                    if rd_full == 0 {
                        mk(Op::Ebreak, 0, 0, 0, 0, 2)
                    } else {
                        mk(Op::Jalr, 1, rd_full, 0, 0, 2)
                    }
                } else {
                    mk(Op::Add, rd_full, rd_full, rs2_full, 0, 2)
                }
            }
            6 => {
                let off = (bits(h, 12, 9) << 2) | (bits(h, 8, 7) << 6);
                mk(Op::Sw, 0, 2, rs2_full, off as i32, 2)
            }
            _ => bad(),
        },
    }
}
