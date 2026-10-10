//! Disassembler producing the text style of `llvm-objdump` for `riscv32` (ABI register names,
//! pseudo-instructions such as `li`/`mv`/`j`/`ret`/`nop`, immediates in hex with a `-0x` sign,
//! absolute branch targets). Compressed instructions print as their expanded base instruction,
//! like objdump does.

use super::csr_names::CSR_NAMES;
use super::decode::{decode, Insn, Op};

const REGS: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7",
    "s8", "s9", "s10", "s11", "t3", "t4", "t5", "t6",
];

/// ABI name of register `x<r>` (`r` is masked to 0-31).
#[inline]
pub fn reg_name(r: u8) -> &'static str {
    REGS[(r & 31) as usize]
}

/// Name of a standard CSR as printed by `llvm-objdump` (`None` for unnamed numbers).
pub fn csr_name(csr: u16) -> Option<&'static str> {
    CSR_NAMES.binary_search_by_key(&csr, |e| e.0).ok().map(|k| CSR_NAMES[k].1)
}

/// Decodes and disassembles the instruction at `pc` held in `word` (low 16 bits for a compressed
/// instruction). Returns the length in bytes and the text.
pub fn disassemble(word: u32, pc: u32) -> (u8, String) {
    let insn = decode(word);
    if insn.len == 2 {
        if let Some(text) = compressed_special(word & 0xffff) {
            return (2, text);
        }
    }
    (insn.len, format_insn(&insn, pc))
}

/// objdump prints compressed HINT encodings and the Zihintntl hints in their `c.` form (the
/// expanded alias would be misleading); everything else prints expanded.
fn compressed_special(h: u32) -> Option<String> {
    let rd = (h >> 7) & 31;
    let rs2 = (h >> 2) & 31;
    let imm6 = || ((((h >> 12) & 1) << 5 | rs2) as i32) << 26 >> 26;
    let shamt = (((h >> 12) & 1) << 5) | rs2;
    let r = |n: u32| reg_name(n as u8);
    match (h & 3, h >> 13) {
        (1, 0) if rd == 0 && imm6() != 0 => Some(format!("c.nop {}", hex(imm6()))),
        (1, 0) if rd != 0 && imm6() == 0 => Some(format!("c.addi {}, 0x0", r(rd))),
        (1, 2) if rd == 0 => Some(format!("c.li zero, {}", hex(imm6()))),
        (1, 3) if rd == 0 && imm6() != 0 => Some(format!("c.lui zero, {:#x}", imm6() as u32 & 0xf_ffff)),
        (1, 4) if (h >> 10) & 3 < 2 && shamt == 0 => {
            let m = if (h >> 10) & 3 == 0 { "c.srli" } else { "c.srai" };
            Some(format!("{m} {}, 0x0", r(8 + ((h >> 7) & 7))))
        }
        (2, 0) if (rd == 0 || shamt == 0) && shamt < 32 => Some(format!("c.slli {}, {:#x}", r(rd), shamt)),
        (2, 4) if (h >> 12) & 1 == 0 && rs2 != 0 && rd == 0 => Some(format!("c.mv zero, {}", r(rs2))),
        (2, 4) if (h >> 12) & 1 == 1 && rs2 != 0 && rd == 0 => Some(match rs2 {
            2 => "c.ntl.p1".to_string(),
            3 => "c.ntl.pall".to_string(),
            4 => "c.ntl.s1".to_string(),
            5 => "c.ntl.all".to_string(),
            _ => format!("c.add zero, {}", r(rs2)),
        }),
        _ => None,
    }
}

fn hex(v: i32) -> String {
    if v < 0 {
        format!("-{:#x}", -(v as i64))
    } else {
        format!("{v:#x}")
    }
}

fn csr_text(csr: i32) -> String {
    match csr_name(csr as u16) {
        Some(n) => n.to_string(),
        None => format!("{csr:#x}"),
    }
}

fn fence_set(bits: u32) -> String {
    let mut s = String::new();
    for (bit, ch) in [(8, 'i'), (4, 'o'), (2, 'r'), (1, 'w')] {
        if bits & bit != 0 {
            s.push(ch);
        }
    }
    if s.is_empty() {
        s.push('0');
    }
    s
}

/// Text of a decoded instruction located at `pc` (needed for branch / jump targets).
pub fn format_insn(i: &Insn, pc: u32) -> String {
    let rd = reg_name(i.rd);
    let rs1 = reg_name(i.rs1);
    let rs2 = reg_name(i.rs2);
    let imm = i.imm;
    let target = || format!("{:#x}", pc.wrapping_add(imm as u32));
    let rrr = |m: &str| format!("{m} {rd}, {rs1}, {rs2}");
    let rri = |m: &str| format!("{m} {rd}, {rs1}, {}", hex(imm));
    let load = |m: &str| format!("{m} {rd}, {}({rs1})", hex(imm));
    let store = |m: &str| format!("{m} {rs2}, {}({rs1})", hex(imm));
    let branch = |m: &str| format!("{m} {rs1}, {rs2}, {}", target());
    match i.op {
        Op::Undecoded => "<undecoded>".to_string(),
        Op::Illegal => {
            match imm as u32 {
                0x1020_0073 => "sret".to_string(),
                0 if i.len == 2 => "unimp".to_string(),
                _ => "<unknown>".to_string(),
            }
        }
        Op::Lui => format!("lui {rd}, {:#x}", (imm as u32) >> 12),
        Op::Auipc if i.rd == 0 => format!("lpad {:#x}", (imm as u32) >> 12),
        Op::Auipc => format!("auipc {rd}, {:#x}", (imm as u32) >> 12),
        Op::Jal => match i.rd {
            0 => format!("j {}", target()),
            1 => format!("jal {}", target()),
            _ => format!("jal {rd}, {}", target()),
        },
        Op::Jalr => match (i.rd, imm) {
            (0, 0) if i.rs1 == 1 => "ret".to_string(),
            (0, 0) => format!("jr {rs1}"),
            (1, 0) => format!("jalr {rs1}"),
            (0, _) => format!("jr {}({rs1})", hex(imm)),
            (1, _) => format!("jalr {}({rs1})", hex(imm)),
            (_, 0) => format!("jalr {rd}, {rs1}"),
            _ => format!("jalr {rd}, {}({rs1})", hex(imm)),
        },
        Op::Beq | Op::Bne | Op::Blt | Op::Bge | Op::Bltu | Op::Bgeu => {
            let m = match i.op {
                Op::Beq => "beq",
                Op::Bne => "bne",
                Op::Blt => "blt",
                Op::Bge => "bge",
                Op::Bltu => "bltu",
                _ => "bgeu",
            };
            match (i.op, i.rs1, i.rs2) {
                (Op::Beq, _, 0) => format!("beqz {rs1}, {}", target()),
                (Op::Bne, _, 0) => format!("bnez {rs1}, {}", target()),
                (Op::Bge, 0, _) => format!("blez {rs2}, {}", target()),
                (Op::Bge, _, 0) => format!("bgez {rs1}, {}", target()),
                (Op::Blt, _, 0) => format!("bltz {rs1}, {}", target()),
                (Op::Blt, 0, _) => format!("bgtz {rs2}, {}", target()),
                _ => branch(m),
            }
        }
        Op::Lb => load("lb"),
        Op::Lh => load("lh"),
        Op::Lw => load("lw"),
        Op::Lbu => load("lbu"),
        Op::Lhu => load("lhu"),
        Op::Sb => store("sb"),
        Op::Sh => store("sh"),
        Op::Sw => store("sw"),
        Op::Addi => {
            if i.rs1 == 0 {
                if i.rd == 0 && imm == 0 {
                    "nop".to_string()
                } else {
                    format!("li {rd}, {}", hex(imm))
                }
            } else if imm == 0 {
                format!("mv {rd}, {rs1}")
            } else {
                rri("addi")
            }
        }
        Op::Slti => rri("slti"),
        Op::Sltiu => {
            if imm == 1 {
                format!("seqz {rd}, {rs1}")
            } else {
                rri("sltiu")
            }
        }
        Op::Xori => {
            if imm == -1 {
                format!("not {rd}, {rs1}")
            } else {
                rri("xori")
            }
        }
        Op::Ori if i.rd == 0 && matches!(imm & 31, 0 | 1 | 3) => {
            let m = match imm & 31 {
                0 => "prefetch.i",
                1 => "prefetch.r",
                _ => "prefetch.w",
            };
            format!("{m} {}({rs1})", hex(imm & !31))
        }
        Op::Ori => rri("ori"),
        Op::Andi => rri("andi"),
        Op::Slli => format!("slli {rd}, {rs1}, {}", hex(imm)),
        Op::Srli => format!("srli {rd}, {rs1}, {}", hex(imm)),
        Op::Srai => format!("srai {rd}, {rs1}, {}", hex(imm)),
        Op::Add if i.rd == 0 && i.rs1 == 0 && (2..=5).contains(&i.rs2) => {
            ["ntl.p1", "ntl.pall", "ntl.s1", "ntl.all"][i.rs2 as usize - 2].to_string()
        }
        Op::Add if i.rs1 == 0 && i.len == 2 => format!("mv {rd}, {rs2}"),
        Op::Add => rrr("add"),
        Op::Sub => {
            if i.rs1 == 0 {
                format!("neg {rd}, {rs2}")
            } else {
                rrr("sub")
            }
        }
        Op::Sll => rrr("sll"),
        Op::Slt => match (i.rs1, i.rs2) {
            (_, 0) => format!("sltz {rd}, {rs1}"),
            (0, _) => format!("sgtz {rd}, {rs2}"),
            _ => rrr("slt"),
        },
        Op::Sltu => {
            if i.rs1 == 0 {
                format!("snez {rd}, {rs2}")
            } else {
                rrr("sltu")
            }
        }
        Op::Xor => rrr("xor"),
        Op::Srl => rrr("srl"),
        Op::Sra => rrr("sra"),
        Op::Or => rrr("or"),
        Op::And => rrr("and"),
        Op::Mul => rrr("mul"),
        Op::Mulh => rrr("mulh"),
        Op::Mulhsu => rrr("mulhsu"),
        Op::Mulhu => rrr("mulhu"),
        Op::Div => rrr("div"),
        Op::Divu => rrr("divu"),
        Op::Rem => rrr("rem"),
        Op::Remu => rrr("remu"),
        Op::Fence => {
            let f = imm as u32;
            let (fm, pred, succ) = (f >> 8, (f >> 4) & 15, f & 15);
            if fm == 8 && pred == 3 && succ == 3 {
                "fence.tso".to_string()
            } else if fm == 0 && pred == 15 && succ == 15 {
                "fence".to_string()
            } else if f == 0x010 {
                "pause".to_string()
            } else {
                format!("fence {}, {}", fence_set(pred), fence_set(succ))
            }
        }
        Op::FenceI => "fence.i".to_string(),
        Op::Ecall => "ecall".to_string(),
        Op::Ebreak => "ebreak".to_string(),
        Op::Mret => "mret".to_string(),
        Op::Wfi => "wfi".to_string(),
        Op::Csrrw | Op::Csrrs | Op::Csrrc | Op::Csrrwi | Op::Csrrsi | Op::Csrrci => {
            let csr = csr_text(imm);
            let reg_form = matches!(i.op, Op::Csrrw | Op::Csrrs | Op::Csrrc);
            let src = if reg_form { rs1.to_string() } else { format!("{:#x}", i.rs1) };
            match (i.op, i.rd, i.rs1) {
                (Op::Csrrw, 0, 0) if imm == 0xc00 => "unimp".to_string(),
                (Op::Csrrs, _, 0) => match imm as u16 {
                    0xc00 | 0xc01 | 0xc02 | 0xc80 | 0xc81 | 0xc82 => format!("rd{} {rd}", csr),
                    _ => format!("csrr {rd}, {csr}"),
                },
                (Op::Csrrw, 0, _) => format!("csrw {csr}, {src}"),
                (Op::Csrrs, 0, _) => format!("csrs {csr}, {src}"),
                (Op::Csrrc, 0, _) => format!("csrc {csr}, {src}"),
                (Op::Csrrwi, 0, _) => format!("csrwi {csr}, {src}"),
                (Op::Csrrsi, 0, _) => format!("csrsi {csr}, {src}"),
                (Op::Csrrci, 0, _) => format!("csrci {csr}, {src}"),
                _ => {
                    let m = match i.op {
                        Op::Csrrw => "csrrw",
                        Op::Csrrs => "csrrs",
                        Op::Csrrc => "csrrc",
                        Op::Csrrwi => "csrrwi",
                        Op::Csrrsi => "csrrsi",
                        _ => "csrrci",
                    };
                    format!("{m} {rd}, {csr}, {src}")
                }
            }
        }
    }
}
