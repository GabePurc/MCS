//! Thumb disassembler producing UAL text close to `llvm-objdump -d --triple=thumbv7em`.
//!
//! It formats the [`Insn`] produced by [`super::thumb::decode`], so the listing always agrees with
//! what the simulator executes. Branch targets are absolute addresses (`b 0x1234`). An
//! [`Disassembler`] tracks IT blocks: instructions inside one get their condition suffix and the
//! 16-bit flag-setting data-processing encodings drop the `s` suffix, as in UAL.
//! Reference: ARM DDI 0403E.e chapter A7 (instruction descriptions, assembler syntax).

use super::thumb::*;

const REGS: [&str; 16] = ["r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "sp", "lr", "pc"];
const CONDS: [&str; 16] = ["eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", ""];

pub fn reg_name(r: u8) -> &'static str {
    REGS[(r & 15) as usize]
}

pub fn cond_name(c: u8) -> &'static str {
    CONDS[(c & 15) as usize]
}

/// Special-register name of an MRS/MSR `SYSm` value.
pub fn sysm_name(sysm: u32) -> Option<&'static str> {
    Some(match sysm {
        0 => "apsr",
        1 => "iapsr",
        2 => "eapsr",
        3 => "xpsr",
        5 => "ipsr",
        6 => "epsr",
        7 => "iepsr",
        8 => "msp",
        9 => "psp",
        16 => "primask",
        17 => "basepri",
        18 => "basepri_max",
        19 => "faultmask",
        20 => "control",
        _ => return None,
    })
}

/// Advances ITSTATE after an instruction in the IT block (ITAdvance).
#[inline]
pub fn it_advance(it: u8) -> u8 {
    if it & 7 == 0 {
        0
    } else {
        (it & 0xe0) | ((it << 1) & 0x1f)
    }
}

/// Stateful disassembler (tracks the IT block).
#[derive(Default)]
pub struct Disassembler {
    it: u8,
    pub features: ArmFeatures,
}

impl Disassembler {
    pub fn new(features: ArmFeatures) -> Self {
        Self { it: 0, features }
    }

    /// Decodes and formats the instruction at `bytes[0..]` located at `addr`.
    /// Returns (length in bytes, text); `None` when `bytes` is too short.
    pub fn next(&mut self, bytes: &[u8], addr: u32) -> Option<(usize, String)> {
        if bytes.len() < 2 {
            return None;
        }
        let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]);
        let hw2 = if is_32bit(hw1) {
            if bytes.len() < 4 {
                return None;
            }
            u16::from_le_bytes([bytes[2], bytes[3]])
        } else {
            0
        };
        let insn = decode(hw1, hw2, self.features);
        Some((insn.len as usize, self.format(&insn, addr)))
    }

    /// Formats `insn` and advances the IT state.
    pub fn format(&mut self, insn: &Insn, addr: u32) -> String {
        let in_it = self.it != 0;
        let cond = if in_it && insn.op != Op::IT { Some(self.it >> 4) } else { None };
        let text = format_insn(insn, addr, cond, in_it);
        if insn.op == Op::IT {
            self.it = insn.imm as u8;
        } else if in_it {
            self.it = it_advance(self.it);
        }
        text
    }
}

/// Formats one instruction. `cond` is the IT condition applying to it (if inside an IT block).
pub fn format_insn(i: &Insn, addr: u32, cond: Option<u8>, in_it: bool) -> String {
    let mut m = String::with_capacity(48);
    let name = OP_NAMES[i.op as usize];
    let wide = i.len == 4;
    let r = reg_name;
    let setf = i.s == S_YES || (i.s == S_NOT_IT && !in_it);
    let mut ops = String::new();
    // (mnemonic, flag suffix applies, wide suffix applies)
    let mut mn = name.to_string();
    let mut flags = false;
    let mut wsfx = false;
    match i.op {
        Op::UNDEF => {
            mn = "<undefined>".into();
        }
        Op::UDF => {
            wsfx = true;
            ops = format!("#{}", i.imm);
        }
        Op::AND_I | Op::BIC_I | Op::ORR_I | Op::ORN_I | Op::EOR_I | Op::ADC_I | Op::SBC_I | Op::RSB_I => {
            flags = true;
            wsfx = i.op == Op::RSB_I;
            if i.op == Op::RSB_I && !wide {
                ops = format!("{}, {}, #0", r(i.rd), r(i.rn));
            } else {
                ops = format!("{}, {}, #{}", r(i.rd), r(i.rn), imm_s(i.imm));
            }
        }
        Op::ADD_I | Op::SUB_I => {
            flags = true;
            if i.ra == 1 {
                // ADDW / SUBW (12-bit plain immediate).
                mn = format!("{}w", name);
                flags = false;
                ops = format!("{}, {}, #{}", r(i.rd), r(i.rn), imm_s(i.imm));
            } else if i.ra == 2 {
                ops = format!("{}, #{}", r(i.rd), imm_s(i.imm));
            } else {
                wsfx = true;
                ops = format!("{}, {}, #{}", r(i.rd), r(i.rn), imm_s(i.imm));
            }
        }
        Op::MOV_I | Op::MVN_I => {
            if i.ra == 1 {
                mn = "movw".into();
                ops = format!("{}, #{}", r(i.rd), imm_s(i.imm));
            } else {
                flags = true;
                wsfx = wide && i.op == Op::MOV_I;
                ops = format!("{}, #{}", r(i.rd), imm_s(i.imm));
            }
        }
        Op::TST_I | Op::TEQ_I | Op::CMP_I | Op::CMN_I => {
            wsfx = true;
            ops = format!("{}, #{}", r(i.rn), imm_s(i.imm));
        }
        Op::AND_R | Op::BIC_R | Op::ORR_R | Op::ORN_R | Op::EOR_R | Op::ADD_R | Op::ADC_R | Op::SUB_R | Op::SBC_R | Op::RSB_R => {
            flags = true;
            wsfx = !matches!(i.op, Op::ORN_R | Op::RSB_R);
            let sh = shift_s(i);
            if i.ra == 2 && i.rm == 13 && i.rd != 13 {
                ops = format!("{}, sp, {}", r(i.rd), r(i.rd));
            } else if i.ra == 2 || (!wide && !matches!(i.op, Op::ADD_R | Op::SUB_R)) {
                ops = format!("{}, {}", r(i.rd), r(i.rm));
            } else {
                ops = format!("{}, {}, {}{}", r(i.rd), r(i.rn), r(i.rm), sh);
            }
        }
        Op::MOV_R | Op::MVN_R => {
            let sh = i.shift;
            let named = i.op == Op::MOV_R && !(sh == SH_LSL && i.amt == 0);
            if named {
                mn = ["lsl", "lsr", "asr", "ror", "rrx"][sh as usize].into();
                ops = if sh == SH_RRX {
                    format!("{}, {}", r(i.rd), r(i.rm))
                } else {
                    format!("{}, {}, #{}", r(i.rd), r(i.rm), i.amt)
                };
            } else {
                ops = format!("{}, {}{}", r(i.rd), r(i.rm), if i.op == Op::MVN_R { shift_s(i) } else { String::new() });
            }
            flags = true;
            wsfx = wide && !(sh == SH_RRX && i.op == Op::MOV_R);
        }
        Op::TST_R | Op::TEQ_R | Op::CMP_R | Op::CMN_R => {
            wsfx = true;
            ops = format!("{}, {}{}", r(i.rn), r(i.rm), shift_s(i));
        }
        Op::LSL_RV | Op::LSR_RV | Op::ASR_RV | Op::ROR_RV => {
            flags = true;
            wsfx = true;
            ops = if wide { format!("{}, {}, {}", r(i.rd), r(i.rn), r(i.rm)) } else { format!("{}, {}", r(i.rd), r(i.rm)) };
        }
        Op::MOVT => ops = format!("{}, #{}", r(i.rd), imm_s(i.imm)),
        Op::ADR => {
            wsfx = wide;
            ops = format!("{}, #{}", r(i.rd), simm_s(i.imm as i32));
        }
        Op::MUL => {
            flags = !wide;
            ops = format!("{}, {}, {}", r(i.rd), r(i.rn), r(i.rm));
        }
        Op::MLA | Op::MLS => ops = format!("{}, {}, {}, {}", r(i.rd), r(i.rn), r(i.rm), r(i.ra)),
        Op::UMULL | Op::SMULL | Op::UMLAL | Op::SMLAL => ops = format!("{}, {}, {}, {}", r(i.rd), r(i.ra), r(i.rn), r(i.rm)),
        Op::SDIV | Op::UDIV => ops = format!("{}, {}, {}", r(i.rd), r(i.rn), r(i.rm)),
        Op::BFC => ops = format!("{}, #{}, #{}", r(i.rd), i.amt, i.aux),
        Op::BFI | Op::UBFX | Op::SBFX => ops = format!("{}, {}, #{}, #{}", r(i.rd), r(i.rn), i.amt, i.aux),
        Op::CLZ | Op::RBIT => ops = format!("{}, {}", r(i.rd), r(i.rm)),
        Op::REV | Op::REV16 | Op::REVSH => {
            wsfx = true;
            ops = format!("{}, {}", r(i.rd), r(i.rm));
        }
        Op::SXTB | Op::SXTH | Op::UXTB | Op::UXTH => {
            let rot = if i.amt != 0 { format!(", ror #{}", i.amt) } else { String::new() };
            if i.rn == 15 {
                wsfx = true;
                ops = format!("{}, {}{}", r(i.rd), r(i.rm), rot);
            } else {
                mn = format!("{}a{}", &name[..3], &name[3..]);
                ops = format!("{}, {}, {}{}", r(i.rd), r(i.rn), r(i.rm), rot);
            }
        }
        Op::SSAT | Op::USAT => {
            let sh = if i.amt == 0 { String::new() } else { format!(", {} #{}", if i.shift == SH_ASR { "asr" } else { "lsl" }, i.amt) };
            ops = format!("{}, #{}, {}{}", r(i.rd), i.imm, r(i.rn), sh);
        }
        Op::QADD | Op::QSUB | Op::QDADD | Op::QDSUB => ops = format!("{}, {}, {}", r(i.rd), r(i.rm), r(i.rn)),
        Op::LDR | Op::LDRB | Op::LDRH | Op::LDRSB | Op::LDRSH | Op::STR | Op::STRB | Op::STRH => {
            wsfx = wide && !matches!(i.aux, AM_PRE | AM_POST) && !(i.aux == AM_OFFSET && (i.imm as i32) < 0);
            ops = format!("{}, {}", r(i.rd), mem_s(i));
        }
        Op::LDRT | Op::LDRBT | Op::LDRHT | Op::LDRSBT | Op::LDRSHT | Op::STRT | Op::STRBT | Op::STRHT => {
            ops = format!("{}, [{}{}]", r(i.rd), r(i.rn), imm_off(i.imm));
        }
        Op::LDRD | Op::STRD => {
            let mut j = *i;
            if j.aux == AM_LIT {
                j.aux = AM_OFFSET;
            }
            ops = format!("{}, {}, {}", r(i.rd), r(i.ra), mem_s(&j));
        }
        Op::PLD | Op::PLI => {
            wsfx = false;
            ops = mem_s(i);
        }
        Op::LDM | Op::STM | Op::LDMDB | Op::STMDB => {
            wsfx = wide && matches!(i.op, Op::LDM | Op::STM);
            ops = format!("{}{}, {}", r(i.rn), if i.aux != 0 { "!" } else { "" }, list_s(i.imm));
        }
        Op::PUSH | Op::POP => {
            wsfx = wide;
            ops = list_s(i.imm);
        }
        Op::LDREX => ops = format!("{}, [{}{}]", r(i.rd), r(i.rn), imm_off(i.imm)),
        Op::LDREXB | Op::LDREXH => ops = format!("{}, [{}]", r(i.rd), r(i.rn)),
        Op::STREX => ops = format!("{}, {}, [{}{}]", r(i.rd), r(i.rm), r(i.rn), imm_off(i.imm)),
        Op::STREXB | Op::STREXH => ops = format!("{}, {}, [{}]", r(i.rd), r(i.rm), r(i.rn)),
        Op::CLREX => {}
        Op::B => {
            wsfx = wide;
            ops = target_s(addr, i.imm);
        }
        Op::B_COND => {
            mn = format!("b{}", cond_name(cond.unwrap_or(i.aux)));
            wsfx = wide;
            ops = target_s(addr, i.imm);
        }
        Op::BL => ops = target_s(addr, i.imm),
        Op::BX | Op::BLX_R => ops = r(i.rm).to_string(),
        Op::CBZ | Op::CBNZ => ops = format!("{}, {}", r(i.rn), target_s(addr, i.imm)),
        Op::TBB => ops = format!("[{}, {}]", r(i.rn), r(i.rm)),
        Op::TBH => ops = format!("[{}, {}, lsl #1]", r(i.rn), r(i.rm)),
        Op::MRS => {
            ops = format!("{}, {}", r(i.rd), sysm_name(i.imm).map(String::from).unwrap_or_else(|| format!("#{}", i.imm)));
        }
        Op::MSR => {
            let base = sysm_name(i.imm).unwrap_or("?");
            let sfx = if i.imm < 4 {
                match i.aux {
                    2 => "_nzcvq",
                    1 => "_g",
                    3 => "_nzcvqg",
                    _ => "",
                }
            } else {
                ""
            };
            ops = format!("{}{}, {}", base, sfx, r(i.rn));
        }
        Op::CPS => {
            mn = if i.aux != 0 { "cpsid".into() } else { "cpsie".into() };
            wsfx = wide;
            ops = format!("{}{}", if i.imm & 2 != 0 { "i" } else { "" }, if i.imm & 1 != 0 { "f" } else { "" });
        }
        Op::SVC | Op::BKPT => ops = format!("#{}", i.imm),
        Op::NOP | Op::YIELD | Op::WFE | Op::WFI | Op::SEV => wsfx = wide,
        Op::DBG => ops = format!("#{}", i.imm),
        Op::DMB | Op::DSB | Op::ISB => {
            ops = match i.imm {
                15 => "sy".into(),
                14 => "st".into(),
                11 => "ish".into(),
                10 => "ishst".into(),
                7 => "nsh".into(),
                6 => "nshst".into(),
                3 => "osh".into(),
                2 => "oshst".into(),
                n => format!("#{}", n),
            }
        }
        Op::IT => {
            let first = (i.imm >> 4) as u8 & 0xf;
            let mask = (i.imm & 0xf) as u8;
            let slots = 4 - mask.trailing_zeros() as usize;
            mn = String::from("it");
            for k in 1..slots {
                let bit = (mask >> (4 - k)) & 1;
                mn.push(if bit == first & 1 { 't' } else { 'e' });
            }
            ops = cond_name(first).to_string();
        }
    }
    m.push_str(&mn);
    if flags && setf {
        m.push('s');
    }
    if let Some(c) = cond {
        if i.op != Op::B_COND {
            m.push_str(cond_name(c));
        }
    }
    if wsfx && wide {
        m.push_str(".w");
    }
    if !ops.is_empty() {
        m.push('\t');
        m.push_str(&ops);
    }
    m
}

/// Decimal for small values, hex otherwise (like LLVM); negative values keep their sign.
fn imm_s(v: u32) -> String {
    if v < 10 {
        v.to_string()
    } else {
        format!("0x{:x}", v)
    }
}

fn simm_s(v: i32) -> String {
    if v < 0 {
        format!("-{}", imm_s(v.unsigned_abs()))
    } else {
        imm_s(v as u32)
    }
}

fn imm_off(v: u32) -> String {
    if v == 0 {
        String::new()
    } else {
        format!(", #{}", simm_s(v as i32))
    }
}

fn shift_s(i: &Insn) -> String {
    match i.shift {
        SH_LSL if i.amt == 0 => String::new(),
        SH_LSL => format!(", lsl #{}", i.amt),
        SH_LSR => format!(", lsr #{}", i.amt),
        SH_ASR => format!(", asr #{}", i.amt),
        SH_ROR => format!(", ror #{}", i.amt),
        _ => ", rrx".to_string(),
    }
}

fn mem_s(i: &Insn) -> String {
    let rn = reg_name(i.rn);
    match i.aux {
        AM_OFFSET => format!("[{}{}]", rn, imm_off(i.imm)),
        AM_LIT => format!("[{}, #{}]", rn, simm_s(i.imm as i32)),
        AM_PRE => format!("[{}, #{}]!", rn, simm_s(i.imm as i32)),
        AM_POST => format!("[{}], #{}", rn, simm_s(i.imm as i32)),
        _ => {
            if i.amt == 0 {
                format!("[{}, {}]", rn, reg_name(i.rm))
            } else {
                format!("[{}, {}, lsl #{}]", rn, reg_name(i.rm), i.amt)
            }
        }
    }
}

fn list_s(list: u32) -> String {
    let mut s = String::from("{");
    let mut first = true;
    for n in 0..16u8 {
        if list & (1 << n) != 0 {
            if !first {
                s.push_str(", ");
            }
            first = false;
            s.push_str(reg_name(n));
        }
    }
    s.push('}');
    s
}

fn target_s(addr: u32, imm: u32) -> String {
    format!("0x{:x}", addr.wrapping_add(4).wrapping_add(imm))
}

/// Disassembles a byte stream starting at `addr` (no IT-block state is carried across calls).
pub fn disassemble(bytes: &[u8], addr: u32, features: ArmFeatures) -> Vec<(u32, usize, String)> {
    let mut d = Disassembler::new(features);
    let mut out = Vec::new();
    let mut off = 0usize;
    while let Some((len, text)) = d.next(&bytes[off..], addr + off as u32) {
        out.push((addr + off as u32, len, text));
        off += len;
    }
    out
}
