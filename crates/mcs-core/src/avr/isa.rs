//! AVR instruction set definition.
//!
//! A single declarative table drives the decoder (64K lookup), the disassembler and the
//! assembler, so adding a core variant or instruction only requires touching this file.
//! Encodings follow the Microchip "AVR Instruction Set Manual" bit patterns: letters mark
//! operand bits and are gathered MSB -> LSB to build the operand value.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Core feature flags. A device's `features` mask is matched against each instruction's req/excl.
pub mod feature {
    /// Reduced core (AVRrc): r16..r31 only, 16-bit LDS/STS, no LDD/STD/ADIW/LPM.
    pub const RC: u32 = 1 << 0;
    pub const MOVW: u32 = 1 << 1;
    pub const MUL: u32 = 1 << 2;
    /// LPM Rd, Z / LPM Rd, Z+
    pub const LPMX: u32 = 1 << 3;
    pub const JMP: u32 = 1 << 4;
    pub const ELPM: u32 = 1 << 5;
    pub const ELPMX: u32 = 1 << 6;
    pub const SPM: u32 = 1 << 7;
    pub const SPMX: u32 = 1 << 8;
    pub const EIJMP: u32 = 1 << 9;
    pub const BREAK: u32 = 1 << 10;
    pub const DES: u32 = 1 << 11;
    /// XCH / LAS / LAC / LAT
    pub const RMW: u32 = 1 << 12;
}

/// Operation ids (dense, used by the executor's `match`).
pub mod op {
    pub const INVALID: u8 = 0;
    pub const NOP: u8 = 1;
    pub const MOVW: u8 = 2;
    pub const MULS: u8 = 3;
    pub const MULSU: u8 = 4;
    pub const FMUL: u8 = 5;
    pub const FMULS: u8 = 6;
    pub const FMULSU: u8 = 7;
    pub const CPC: u8 = 8;
    pub const SBC: u8 = 9;
    pub const ADD: u8 = 10;
    pub const CPSE: u8 = 11;
    pub const CP: u8 = 12;
    pub const SUB: u8 = 13;
    pub const ADC: u8 = 14;
    pub const AND: u8 = 15;
    pub const EOR: u8 = 16;
    pub const OR: u8 = 17;
    pub const MOV: u8 = 18;
    pub const CPI: u8 = 19;
    pub const SBCI: u8 = 20;
    pub const SUBI: u8 = 21;
    pub const ORI: u8 = 22;
    pub const ANDI: u8 = 23;
    pub const LD_Y: u8 = 24;
    pub const LD_Z: u8 = 25;
    pub const ST_Y: u8 = 26;
    pub const ST_Z: u8 = 27;
    pub const LDD_Y: u8 = 28;
    pub const LDD_Z: u8 = 29;
    pub const STD_Y: u8 = 30;
    pub const STD_Z: u8 = 31;
    pub const LDS_RC: u8 = 32;
    pub const STS_RC: u8 = 33;
    pub const LDS: u8 = 34;
    pub const LD_ZP: u8 = 35;
    pub const LD_MZ: u8 = 36;
    pub const LPM_Z: u8 = 37;
    pub const LPM_ZP: u8 = 38;
    pub const ELPM_Z: u8 = 39;
    pub const ELPM_ZP: u8 = 40;
    pub const LD_YP: u8 = 41;
    pub const LD_MY: u8 = 42;
    pub const LD_X: u8 = 43;
    pub const LD_XP: u8 = 44;
    pub const LD_MX: u8 = 45;
    pub const POP: u8 = 46;
    pub const STS: u8 = 47;
    pub const ST_ZP: u8 = 48;
    pub const ST_MZ: u8 = 49;
    pub const XCH: u8 = 50;
    pub const LAS: u8 = 51;
    pub const LAC: u8 = 52;
    pub const LAT: u8 = 53;
    pub const ST_YP: u8 = 54;
    pub const ST_MY: u8 = 55;
    pub const ST_X: u8 = 56;
    pub const ST_XP: u8 = 57;
    pub const ST_MX: u8 = 58;
    pub const PUSH: u8 = 59;
    pub const COM: u8 = 60;
    pub const NEG: u8 = 61;
    pub const SWAP: u8 = 62;
    pub const INC: u8 = 63;
    pub const ASR: u8 = 64;
    pub const LSR: u8 = 65;
    pub const ROR: u8 = 66;
    pub const DEC: u8 = 67;
    pub const BSET: u8 = 68;
    pub const BCLR: u8 = 69;
    pub const IJMP: u8 = 70;
    pub const EIJMP: u8 = 71;
    pub const DES: u8 = 72;
    pub const RET: u8 = 73;
    pub const RETI: u8 = 74;
    pub const SLEEP: u8 = 75;
    pub const BREAK: u8 = 76;
    pub const WDR: u8 = 77;
    pub const LPM: u8 = 78;
    pub const ELPM: u8 = 79;
    pub const SPM: u8 = 80;
    pub const SPM_ZP: u8 = 81;
    pub const ICALL: u8 = 82;
    pub const EICALL: u8 = 83;
    pub const JMP: u8 = 84;
    pub const CALL: u8 = 85;
    pub const ADIW: u8 = 86;
    pub const SBIW: u8 = 87;
    pub const CBI: u8 = 88;
    pub const SBIC: u8 = 89;
    pub const SBI: u8 = 90;
    pub const SBIS: u8 = 91;
    pub const MUL: u8 = 92;
    pub const IN: u8 = 93;
    pub const OUT: u8 = 94;
    pub const RJMP: u8 = 95;
    pub const RCALL: u8 = 96;
    pub const LDI: u8 = 97;
    pub const BRBS: u8 = 98;
    pub const BRBC: u8 = 99;
    pub const BLD: u8 = 100;
    pub const BST: u8 = 101;
    pub const SBRC: u8 = 102;
    pub const SBRS: u8 = 103;
}

pub const OP_COUNT: usize = 104;

/// Operand kinds. Register kinds yield the register number, immediates the raw value,
/// relative kinds a signed word offset, `K7rc` a data-space address. Pointer kinds are literal
/// tokens (no value) except `YQ` / `ZQ` (`Y+q` / `Z+q`), which yield q.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OperandKind {
    Rd5, Rr5, Rd4, Rr4, Rd3, Rr3, RdW, RrW, RdP,
    K8, K6, K4, A5, A6, B, S,
    K7, K12, K22, K16, K7rc,
    X, XInc, XDec, Y, YInc, YDec, Z, ZInc, ZDec, YQ, ZQ,
}

impl OperandKind {
    /// Literal pointer operands carry no value.
    pub fn is_literal(self) -> bool {
        use OperandKind::*;
        matches!(self, X | XInc | XDec | Y | YInc | YDec | Z | ZInc | ZDec)
    }

    /// Assembly spelling of literal operands.
    pub fn literal_text(self) -> &'static str {
        use OperandKind::*;
        match self {
            X => "X", XInc => "X+", XDec => "-X", Y => "Y", YInc => "Y+", YDec => "-Y",
            Z => "Z", ZInc => "Z+", ZDec => "-Z", YQ => "Y+q", ZQ => "Z+q",
            _ => "",
        }
    }

    fn letter(self) -> u8 {
        use OperandKind::*;
        match self {
            Rd5 | Rd4 | Rd3 | RdW | RdP => b'd',
            Rr5 | Rr4 | Rr3 | RrW => b'r',
            K8 | K6 | K4 => b'K',
            A5 | A6 => b'A',
            B => b'b',
            S => b's',
            K7 | K12 | K22 | K16 | K7rc => b'k',
            YQ | ZQ => b'q',
            _ => 0,
        }
    }

    pub fn is_register(self) -> bool {
        use OperandKind::*;
        matches!(self, Rd5 | Rr5 | Rd4 | Rr4 | Rd3 | Rr3 | RdW | RrW | RdP)
    }
}

#[derive(Clone, Debug)]
pub struct InsnDef {
    pub op: u8,
    /// Canonical lowercase mnemonic.
    pub name: &'static str,
    pub pattern: String,
    pub operands: Vec<OperandKind>,
    /// Features that must all be present.
    pub req: u32,
    /// Features that must all be absent.
    pub excl: u32,
    /// Base cycle count on classic (AVRe) cores.
    pub cycles: u8,
    /// Base cycle count on the reduced core (AVRrc).
    pub cycles_rc: u8,
    pub words: u8,
    /// Fixed-bit mask/value of the first instruction word.
    pub mask: u16,
    pub value: u16,
    /// For each non-literal operand: bit positions (MSB first) in the 16 or 32-bit encoding.
    pub fields: Vec<Vec<u8>>,
}

impl InsnDef {
    pub fn is_available(&self, features: u32) -> bool {
        (self.req & features) == self.req && (self.excl & features) == 0
    }

    /// Non-literal operand kinds, in order (one per value in `encode`/`decode`).
    pub fn value_operands(&self) -> impl Iterator<Item = OperandKind> + '_ {
        self.operands.iter().copied().filter(|k| !k.is_literal())
    }

    /// Gathers the raw (un-normalized) field value (w2 is the second word of 32-bit encodings).
    pub fn extract_field(&self, field: &[u8], w1: u16, w2: u16) -> u32 {
        let full: u32 = if self.words == 2 { ((w1 as u32) << 16) | w2 as u32 } else { w1 as u32 };
        field.iter().fold(0u32, |v, &p| (v << 1) | ((full >> p) & 1))
    }
}

use OperandKind as K;
const RC: u32 = feature::RC;

type Row = (u8, &'static str, &'static str, &'static [OperandKind], u32, u32, u8, u8);

#[rustfmt::skip]
const ROWS: &[Row] = &[
    (op::NOP,    "nop",    "0000 0000 0000 0000", &[], 0, 0, 1, 1),
    (op::MOVW,   "movw",   "0000 0001 dddd rrrr", &[K::RdW, K::RrW], feature::MOVW, 0, 1, 1),
    (op::MULS,   "muls",   "0000 0010 dddd rrrr", &[K::Rd4, K::Rr4], feature::MUL, 0, 2, 2),
    (op::MULSU,  "mulsu",  "0000 0011 0ddd 0rrr", &[K::Rd3, K::Rr3], feature::MUL, 0, 2, 2),
    (op::FMUL,   "fmul",   "0000 0011 0ddd 1rrr", &[K::Rd3, K::Rr3], feature::MUL, 0, 2, 2),
    (op::FMULS,  "fmuls",  "0000 0011 1ddd 0rrr", &[K::Rd3, K::Rr3], feature::MUL, 0, 2, 2),
    (op::FMULSU, "fmulsu", "0000 0011 1ddd 1rrr", &[K::Rd3, K::Rr3], feature::MUL, 0, 2, 2),
    (op::CPC,    "cpc",    "0000 01rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::SBC,    "sbc",    "0000 10rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::ADD,    "add",    "0000 11rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::CPSE,   "cpse",   "0001 00rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::CP,     "cp",     "0001 01rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::SUB,    "sub",    "0001 10rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::ADC,    "adc",    "0001 11rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::AND,    "and",    "0010 00rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::EOR,    "eor",    "0010 01rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::OR,     "or",     "0010 10rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::MOV,    "mov",    "0010 11rd dddd rrrr", &[K::Rd5, K::Rr5], 0, 0, 1, 1),
    (op::CPI,    "cpi",    "0011 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::SBCI,   "sbci",   "0100 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::SUBI,   "subi",   "0101 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::ORI,    "ori",    "0110 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::ANDI,   "andi",   "0111 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::LD_Y,   "ld",     "1000 000d dddd 1000", &[K::Rd5, K::Y], 0, 0, 2, 2),
    (op::LD_Z,   "ld",     "1000 000d dddd 0000", &[K::Rd5, K::Z], 0, 0, 2, 2),
    (op::ST_Y,   "st",     "1000 001r rrrr 1000", &[K::Y, K::Rr5], 0, 0, 2, 2),
    (op::ST_Z,   "st",     "1000 001r rrrr 0000", &[K::Z, K::Rr5], 0, 0, 2, 2),
    (op::LDD_Y,  "ldd",    "10q0 qq0d dddd 1qqq", &[K::Rd5, K::YQ], 0, RC, 2, 2),
    (op::LDD_Z,  "ldd",    "10q0 qq0d dddd 0qqq", &[K::Rd5, K::ZQ], 0, RC, 2, 2),
    (op::STD_Y,  "std",    "10q0 qq1r rrrr 1qqq", &[K::YQ, K::Rr5], 0, RC, 2, 2),
    (op::STD_Z,  "std",    "10q0 qq1r rrrr 0qqq", &[K::ZQ, K::Rr5], 0, RC, 2, 2),
    (op::LDS_RC, "lds",    "1010 0kkk dddd kkkk", &[K::Rd4, K::K7rc], RC, 0, 1, 2),
    (op::STS_RC, "sts",    "1010 1kkk rrrr kkkk", &[K::K7rc, K::Rr4], RC, 0, 1, 2),
    (op::LDS,    "lds",    "1001 000d dddd 0000 kkkk kkkk kkkk kkkk", &[K::Rd5, K::K16], 0, RC, 2, 2),
    (op::LD_ZP,  "ld",     "1001 000d dddd 0001", &[K::Rd5, K::ZInc], 0, 0, 2, 2),
    (op::LD_MZ,  "ld",     "1001 000d dddd 0010", &[K::Rd5, K::ZDec], 0, 0, 2, 2),
    (op::LPM_Z,  "lpm",    "1001 000d dddd 0100", &[K::Rd5, K::Z], feature::LPMX, RC, 3, 3),
    (op::LPM_ZP, "lpm",    "1001 000d dddd 0101", &[K::Rd5, K::ZInc], feature::LPMX, RC, 3, 3),
    (op::ELPM_Z, "elpm",   "1001 000d dddd 0110", &[K::Rd5, K::Z], feature::ELPMX, RC, 3, 3),
    (op::ELPM_ZP,"elpm",   "1001 000d dddd 0111", &[K::Rd5, K::ZInc], feature::ELPMX, RC, 3, 3),
    (op::LD_YP,  "ld",     "1001 000d dddd 1001", &[K::Rd5, K::YInc], 0, 0, 2, 2),
    (op::LD_MY,  "ld",     "1001 000d dddd 1010", &[K::Rd5, K::YDec], 0, 0, 2, 2),
    (op::LD_X,   "ld",     "1001 000d dddd 1100", &[K::Rd5, K::X], 0, 0, 2, 1),
    (op::LD_XP,  "ld",     "1001 000d dddd 1101", &[K::Rd5, K::XInc], 0, 0, 2, 2),
    (op::LD_MX,  "ld",     "1001 000d dddd 1110", &[K::Rd5, K::XDec], 0, 0, 2, 2),
    (op::POP,    "pop",    "1001 000d dddd 1111", &[K::Rd5], 0, 0, 2, 2),
    (op::STS,    "sts",    "1001 001r rrrr 0000 kkkk kkkk kkkk kkkk", &[K::K16, K::Rr5], 0, RC, 2, 2),
    (op::ST_ZP,  "st",     "1001 001r rrrr 0001", &[K::ZInc, K::Rr5], 0, 0, 2, 2),
    (op::ST_MZ,  "st",     "1001 001r rrrr 0010", &[K::ZDec, K::Rr5], 0, 0, 2, 2),
    (op::XCH,    "xch",    "1001 001r rrrr 0100", &[K::Z, K::Rr5], feature::RMW, RC, 2, 2),
    (op::LAS,    "las",    "1001 001r rrrr 0101", &[K::Z, K::Rr5], feature::RMW, RC, 2, 2),
    (op::LAC,    "lac",    "1001 001r rrrr 0110", &[K::Z, K::Rr5], feature::RMW, RC, 2, 2),
    (op::LAT,    "lat",    "1001 001r rrrr 0111", &[K::Z, K::Rr5], feature::RMW, RC, 2, 2),
    (op::ST_YP,  "st",     "1001 001r rrrr 1001", &[K::YInc, K::Rr5], 0, 0, 2, 2),
    (op::ST_MY,  "st",     "1001 001r rrrr 1010", &[K::YDec, K::Rr5], 0, 0, 2, 2),
    (op::ST_X,   "st",     "1001 001r rrrr 1100", &[K::X, K::Rr5], 0, 0, 2, 2),
    (op::ST_XP,  "st",     "1001 001r rrrr 1101", &[K::XInc, K::Rr5], 0, 0, 2, 2),
    (op::ST_MX,  "st",     "1001 001r rrrr 1110", &[K::XDec, K::Rr5], 0, 0, 2, 2),
    (op::PUSH,   "push",   "1001 001r rrrr 1111", &[K::Rr5], 0, 0, 2, 2),
    (op::COM,    "com",    "1001 010d dddd 0000", &[K::Rd5], 0, 0, 1, 1),
    (op::NEG,    "neg",    "1001 010d dddd 0001", &[K::Rd5], 0, 0, 1, 1),
    (op::SWAP,   "swap",   "1001 010d dddd 0010", &[K::Rd5], 0, 0, 1, 1),
    (op::INC,    "inc",    "1001 010d dddd 0011", &[K::Rd5], 0, 0, 1, 1),
    (op::ASR,    "asr",    "1001 010d dddd 0101", &[K::Rd5], 0, 0, 1, 1),
    (op::LSR,    "lsr",    "1001 010d dddd 0110", &[K::Rd5], 0, 0, 1, 1),
    (op::ROR,    "ror",    "1001 010d dddd 0111", &[K::Rd5], 0, 0, 1, 1),
    (op::DEC,    "dec",    "1001 010d dddd 1010", &[K::Rd5], 0, 0, 1, 1),
    (op::BSET,   "bset",   "1001 0100 0sss 1000", &[K::S], 0, 0, 1, 1),
    (op::BCLR,   "bclr",   "1001 0100 1sss 1000", &[K::S], 0, 0, 1, 1),
    (op::IJMP,   "ijmp",   "1001 0100 0000 1001", &[], 0, 0, 2, 2),
    (op::EIJMP,  "eijmp",  "1001 0100 0001 1001", &[], feature::EIJMP, RC, 2, 2),
    (op::DES,    "des",    "1001 0100 KKKK 1011", &[K::K4], feature::DES, RC, 1, 1),
    (op::RET,    "ret",    "1001 0101 0000 1000", &[], 0, 0, 4, 4),
    (op::RETI,   "reti",   "1001 0101 0001 1000", &[], 0, 0, 4, 4),
    (op::SLEEP,  "sleep",  "1001 0101 1000 1000", &[], 0, 0, 1, 1),
    (op::BREAK,  "break",  "1001 0101 1001 1000", &[], feature::BREAK, 0, 1, 1),
    (op::WDR,    "wdr",    "1001 0101 1010 1000", &[], 0, 0, 1, 1),
    (op::LPM,    "lpm",    "1001 0101 1100 1000", &[], 0, RC, 3, 3),
    (op::ELPM,   "elpm",   "1001 0101 1101 1000", &[], feature::ELPM, RC, 3, 3),
    (op::SPM,    "spm",    "1001 0101 1110 1000", &[], feature::SPM, RC, 1, 1),
    (op::SPM_ZP, "spm",    "1001 0101 1111 1000", &[K::ZInc], feature::SPMX, RC, 1, 1),
    (op::ICALL,  "icall",  "1001 0101 0000 1001", &[], 0, 0, 3, 3),
    (op::EICALL, "eicall", "1001 0101 0001 1001", &[], feature::EIJMP, RC, 4, 4),
    (op::JMP,    "jmp",    "1001 010k kkkk 110k kkkk kkkk kkkk kkkk", &[K::K22], feature::JMP, RC, 3, 3),
    (op::CALL,   "call",   "1001 010k kkkk 111k kkkk kkkk kkkk kkkk", &[K::K22], feature::JMP, RC, 4, 4),
    (op::ADIW,   "adiw",   "1001 0110 KKdd KKKK", &[K::RdP, K::K6], 0, RC, 2, 2),
    (op::SBIW,   "sbiw",   "1001 0111 KKdd KKKK", &[K::RdP, K::K6], 0, RC, 2, 2),
    (op::CBI,    "cbi",    "1001 1000 AAAA Abbb", &[K::A5, K::B], 0, 0, 2, 1),
    (op::SBIC,   "sbic",   "1001 1001 AAAA Abbb", &[K::A5, K::B], 0, 0, 1, 1),
    (op::SBI,    "sbi",    "1001 1010 AAAA Abbb", &[K::A5, K::B], 0, 0, 2, 1),
    (op::SBIS,   "sbis",   "1001 1011 AAAA Abbb", &[K::A5, K::B], 0, 0, 1, 1),
    (op::MUL,    "mul",    "1001 11rd dddd rrrr", &[K::Rd5, K::Rr5], feature::MUL, 0, 2, 2),
    (op::IN,     "in",     "1011 0AAd dddd AAAA", &[K::Rd5, K::A6], 0, 0, 1, 1),
    (op::OUT,    "out",    "1011 1AAr rrrr AAAA", &[K::A6, K::Rr5], 0, 0, 1, 1),
    (op::RJMP,   "rjmp",   "1100 kkkk kkkk kkkk", &[K::K12], 0, 0, 2, 2),
    (op::RCALL,  "rcall",  "1101 kkkk kkkk kkkk", &[K::K12], 0, 0, 3, 3),
    (op::LDI,    "ldi",    "1110 KKKK dddd KKKK", &[K::Rd4, K::K8], 0, 0, 1, 1),
    (op::BRBS,   "brbs",   "1111 00kk kkkk ksss", &[K::S, K::K7], 0, 0, 1, 1),
    (op::BRBC,   "brbc",   "1111 01kk kkkk ksss", &[K::S, K::K7], 0, 0, 1, 1),
    (op::BLD,    "bld",    "1111 100d dddd 0bbb", &[K::Rd5, K::B], 0, 0, 1, 1),
    (op::BST,    "bst",    "1111 101d dddd 0bbb", &[K::Rd5, K::B], 0, 0, 1, 1),
    (op::SBRC,   "sbrc",   "1111 110r rrrr 0bbb", &[K::Rr5, K::B], 0, 0, 1, 1),
    (op::SBRS,   "sbrs",   "1111 111r rrrr 0bbb", &[K::Rr5, K::B], 0, 0, 1, 1),
];

fn build_def(row: &Row) -> InsnDef {
    let (op, name, pat, operands, req, excl, cycles, cycles_rc) = *row;
    let bits: Vec<u8> = pat.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    assert!(bits.len() == 16 || bits.len() == 32, "bad pattern for {name}");
    let words = (bits.len() / 16) as u8;
    let (mut mask, mut value) = (0u16, 0u16);
    for (i, &c) in bits.iter().take(16).enumerate() {
        let bit = 15 - i;
        if c == b'0' || c == b'1' {
            mask |= 1 << bit;
            if c == b'1' {
                value |= 1 << bit;
            }
        }
    }
    let fields = operands
        .iter()
        .filter(|k| !k.is_literal())
        .map(|k| {
            let letter = k.letter();
            bits.iter()
                .enumerate()
                .filter(|(_, &c)| c == letter)
                .map(|(i, _)| (bits.len() - 1 - i) as u8)
                .collect::<Vec<u8>>()
        })
        .collect();
    InsnDef { op, name, pattern: String::from_utf8(bits).unwrap(), operands: operands.to_vec(), req, excl, cycles, cycles_rc, words, mask, value, fields }
}

/// All instruction definitions.
pub fn insns() -> &'static [InsnDef] {
    static DEFS: OnceLock<Vec<InsnDef>> = OnceLock::new();
    DEFS.get_or_init(|| ROWS.iter().map(build_def).collect())
}

/// Definition by op id (None for INVALID).
pub fn def_by_op(op: u8) -> Option<&'static InsnDef> {
    static BY_OP: OnceLock<Vec<Option<usize>>> = OnceLock::new();
    let table = BY_OP.get_or_init(|| {
        let mut t = vec![None; OP_COUNT];
        for (i, d) in insns().iter().enumerate() {
            t[d.op as usize] = Some(i);
        }
        t
    });
    table.get(op as usize).copied().flatten().map(|i| &insns()[i])
}

/// Converts a raw field to its operand value (register number, signed offset, address...).
pub fn normalize_operand(kind: OperandKind, raw: u32, width: usize) -> i32 {
    match kind {
        K::Rd4 | K::Rr4 | K::Rd3 | K::Rr3 => raw as i32 + 16,
        K::RdW | K::RrW => raw as i32 * 2,
        K::RdP => 24 + raw as i32 * 2,
        K::K7 | K::K12 => {
            let sign = 1i32 << (width - 1);
            (raw as i32 ^ sign) - sign
        }
        K::K7rc => rc_addr_from_field(raw) as i32,
        _ => raw as i32,
    }
}

/// Inverse of [`normalize_operand`]: returns raw field bits, or None if not encodable.
pub fn denormalize_operand(kind: OperandKind, value: i32, width: usize) -> Option<u32> {
    let max = (1i64 << width) - 1;
    let v = value as i64;
    match kind {
        K::Rd4 | K::Rr4 => (16..=31).contains(&value).then(|| (value - 16) as u32),
        K::Rd3 | K::Rr3 => (16..=23).contains(&value).then(|| (value - 16) as u32),
        K::RdW | K::RrW => ((0..=30).contains(&value) && value & 1 == 0).then_some((value >> 1) as u32),
        K::RdP => ((24..=30).contains(&value) && value & 1 == 0).then(|| ((value - 24) >> 1) as u32),
        K::K7 | K::K12 => {
            let half = 1i64 << (width - 1);
            (v >= -half && v < half).then_some((v & max) as u32)
        }
        K::K7rc => (0x40..=0xbf).contains(&value).then(|| rc_field_from_addr(value as u32)),
        _ => (v >= 0 && v <= max).then_some(v as u32),
    }
}

/// AVRrc 16-bit LDS/STS: gathered k bits (INST[10:8], INST[3:0]) -> data address 0x40..0xBF.
pub fn rc_addr_from_field(v: u32) -> u32 {
    let i8 = (v >> 4) & 1;
    ((i8 ^ 1) << 7) | (i8 << 6) | (((v >> 6) & 1) << 5) | (((v >> 5) & 1) << 4) | (v & 0xf)
}

pub fn rc_field_from_addr(a: u32) -> u32 {
    (((a >> 5) & 1) << 6) | (((a >> 4) & 1) << 5) | (((a >> 6) & 1) << 4) | (a & 0xf)
}

/// Encodes an instruction from normalized operand values (one per non-literal operand).
pub fn encode(def: &InsnDef, values: &[i32]) -> Result<Vec<u16>, String> {
    let mut full: u32 = if def.words == 2 { (def.value as u32) << 16 } else { def.value as u32 };
    for (fi, kind) in def.value_operands().enumerate() {
        let field = &def.fields[fi];
        let v = *values.get(fi).ok_or_else(|| format!("missing operand {}", fi + 1))?;
        let raw = denormalize_operand(kind, v, field.len()).ok_or_else(|| format!("operand {v} out of range for {kind:?}"))?;
        for (i, &pos) in field.iter().enumerate() {
            if (raw >> (field.len() - 1 - i)) & 1 != 0 {
                full |= 1 << pos;
            }
        }
    }
    Ok(if def.words == 2 { vec![(full >> 16) as u16, full as u16] } else { vec![full as u16] })
}

/// Operand value range for diagnostics in the assembler.
pub fn operand_range(kind: OperandKind, features: u32) -> (i32, i32) {
    let rc = features & feature::RC != 0;
    match kind {
        K::Rd5 | K::Rr5 => (if rc { 16 } else { 0 }, 31),
        K::Rd4 | K::Rr4 => (16, 31),
        K::Rd3 | K::Rr3 => (16, 23),
        K::RdW | K::RrW => (0, 30),
        K::RdP => (24, 30),
        K::K8 => (-128, 255),
        K::K6 => (0, 63),
        K::K4 => (0, 15),
        K::A5 => (0, 31),
        K::A6 => (0, 63),
        K::B | K::S => (0, 7),
        K::K7 => (-64, 63),
        K::K12 => (-2048, 2047),
        K::K22 => (0, 0x3f_ffff),
        K::K16 => (0, 0xffff),
        K::K7rc => (0x40, 0xbf),
        K::YQ | K::ZQ => (0, 63),
        _ => (0, 0),
    }
}

/// First-word -> op lookup for one feature set.
pub struct DecodeTable {
    pub features: u32,
    pub ops: Vec<u8>,
}

/// Builds (and caches) the 64K first-word -> op table for a feature set. More specific
/// patterns (more fixed bits) take precedence, so e.g. `LD Rd, Y` wins over `LDD Rd, Y+0`.
pub fn decode_table(features: u32) -> Arc<DecodeTable> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Arc<DecodeTable>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(t) = cache.lock().unwrap().get(&features) {
        return t.clone();
    }
    let rc = features & feature::RC != 0;
    let mut ops = vec![0u8; 65536];
    let mut defs: Vec<&InsnDef> = insns().iter().filter(|d| d.is_available(features)).collect();
    defs.sort_by_key(|d| std::cmp::Reverse(d.mask.count_ones()));
    for def in defs {
        let free = !def.mask;
        // Enumerate all subsets of the free bits (submask enumeration).
        let mut sub = free;
        loop {
            let w = def.value | sub;
            if ops[w as usize] == 0 && (!rc || rc_registers_valid(def, w)) {
                ops[w as usize] = def.op;
            }
            if sub == 0 {
                break;
            }
            sub = (sub - 1) & free;
        }
    }
    let t = Arc::new(DecodeTable { features, ops });
    cache.lock().unwrap().insert(features, t.clone());
    t
}

/// On the reduced core 5-bit register fields must address r16..r31.
fn rc_registers_valid(def: &InsnDef, w: u16) -> bool {
    def.value_operands()
        .enumerate()
        .all(|(fi, k)| !matches!(k, K::Rd5 | K::Rr5) || def.extract_field(&def.fields[fi], w, 0) >= 16)
}

#[derive(Clone, Debug)]
pub struct Decoded {
    pub def: Option<&'static InsnDef>,
    /// Normalized non-literal operand values.
    pub values: Vec<i32>,
    pub words: u8,
}

pub fn decode(table: &DecodeTable, w1: u16, w2: u16) -> Decoded {
    match def_by_op(table.ops[w1 as usize]) {
        None => Decoded { def: None, values: Vec::new(), words: 1 },
        Some(def) => {
            let values = def
                .value_operands()
                .enumerate()
                .map(|(fi, k)| {
                    let f = &def.fields[fi];
                    normalize_operand(k, def.extract_field(f, w1, w2), f.len())
                })
                .collect();
            Decoded { def: Some(def), values, words: def.words }
        }
    }
}

/// SREG bit names, index = bit number.
pub const SREG_BITS: [&str; 8] = ["C", "Z", "N", "V", "S", "H", "T", "I"];

/// Branch aliases: (mnemonic, sreg bit, set(brbs)=true).
pub const BRANCH_ALIASES: &[(&str, u8, bool)] = &[
    ("brcs", 0, true), ("brlo", 0, true), ("brcc", 0, false), ("brsh", 0, false),
    ("breq", 1, true), ("brne", 1, false), ("brmi", 2, true), ("brpl", 2, false),
    ("brvs", 3, true), ("brvc", 3, false), ("brlt", 4, true), ("brge", 4, false),
    ("brhs", 5, true), ("brhc", 5, false), ("brts", 6, true), ("brtc", 6, false),
    ("brie", 7, true), ("brid", 7, false),
];

/// Flag set/clear aliases: (mnemonic, sreg bit, set(bset)=true).
pub const FLAG_ALIASES: &[(&str, u8, bool)] = &[
    ("sec", 0, true), ("clc", 0, false), ("sez", 1, true), ("clz", 1, false),
    ("sen", 2, true), ("cln", 2, false), ("sev", 3, true), ("clv", 3, false),
    ("ses", 4, true), ("cls", 4, false), ("seh", 5, true), ("clh", 5, false),
    ("set", 6, true), ("clt", 6, false), ("sei", 7, true), ("cli", 7, false),
];

fn branch_name(bit: u8, set: bool) -> &'static str {
    BRANCH_ALIASES.iter().find(|a| a.1 == bit && a.2 == set).map(|a| a.0).unwrap_or("brbs")
}

fn flag_name(bit: u8, set: bool) -> &'static str {
    FLAG_ALIASES.iter().find(|a| a.1 == bit && a.2 == set).map(|a| a.0).unwrap_or("bset")
}

/// Name resolvers for disassembly.
#[derive(Default)]
pub struct DisasmContext<'a> {
    /// Code BYTE address -> label.
    pub code_label: Option<&'a dyn Fn(u32) -> Option<String>>,
    /// I/O address (IN/OUT/SBI numbering) -> register name.
    pub io_name: Option<&'a dyn Fn(u32) -> Option<String>>,
    /// Data-space address -> name.
    pub data_name: Option<&'a dyn Fn(u32) -> Option<String>>,
    /// Flash size in bytes, for wrapping relative jumps (0 = 8 MB).
    pub flash_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disasm {
    pub mnemonic: String,
    pub operands: String,
    /// Target code byte address for branches/jumps/calls.
    pub target: Option<u32>,
    pub words: u8,
    pub valid: bool,
}

/// Disassembles one instruction at word address `pc`, using the aliases Atmel Studio shows
/// (BREQ, SEI, CLR, LSL, TST, ROL, SER).
pub fn disassemble(table: &DecodeTable, pc: u32, w1: u16, w2: u16, ctx: &DisasmContext) -> Disasm {
    let d = decode(table, w1, w2);
    let Some(def) = d.def else {
        return Disasm { mnemonic: ".dw".into(), operands: format!("0x{w1:04X}"), target: None, words: 1, valid: false };
    };
    let v = &d.values;
    let flash_words = (if ctx.flash_bytes == 0 { 0x80_0000 } else { ctx.flash_bytes } >> 1) as i64;
    let rel_target = |k: i32| -> u32 { ((((pc as i64 + 1 + k as i64) % flash_words) + flash_words) % flash_words * 2) as u32 };
    let simple = |m: &str, ops: String| Disasm { mnemonic: m.into(), operands: ops, target: None, words: 1, valid: true };
    let reg = |r: i32| format!("r{r}");

    match def.op {
        op::BRBS | op::BRBC => {
            let target = rel_target(v[1]);
            return Disasm {
                mnemonic: branch_name(v[0] as u8, def.op == op::BRBS).into(),
                operands: fmt_target(target, v[1], ctx),
                target: Some(target),
                words: 1,
                valid: true,
            };
        }
        op::BSET | op::BCLR => return simple(flag_name(v[0] as u8, def.op == op::BSET), String::new()),
        op::EOR if v[0] == v[1] => return simple("clr", reg(v[0])),
        op::ADD if v[0] == v[1] => return simple("lsl", reg(v[0])),
        op::ADC if v[0] == v[1] => return simple("rol", reg(v[0])),
        op::AND if v[0] == v[1] => return simple("tst", reg(v[0])),
        op::LDI if v[1] == 0xff => return simple("ser", reg(v[0])),
        _ => {}
    }

    let mut parts = Vec::with_capacity(2);
    let mut target = None;
    let mut fi = 0;
    for &kind in &def.operands {
        if kind.is_literal() {
            parts.push(kind.literal_text().to_string());
            continue;
        }
        let val = v[fi];
        fi += 1;
        parts.push(match kind {
            K::Rd5 | K::Rr5 | K::Rd4 | K::Rr4 | K::Rd3 | K::Rr3 | K::RdP | K::RdW | K::RrW => reg(val),
            K::K8 => format!("0x{val:02X}"),
            K::K6 | K::K4 | K::B | K::S => val.to_string(),
            K::A5 | K::A6 => ctx.io_name.and_then(|f| f(val as u32)).unwrap_or_else(|| format!("0x{val:02X}")),
            K::K12 => {
                let t = rel_target(val);
                target = Some(t);
                fmt_target(t, val, ctx)
            }
            K::K22 => {
                let t = val as u32 * 2;
                target = Some(t);
                ctx.code_label.and_then(|f| f(t)).unwrap_or_else(|| format!("0x{t:04X}"))
            }
            K::K16 | K::K7rc => ctx.data_name.and_then(|f| f(val as u32)).unwrap_or_else(|| format!("0x{val:04X}")),
            K::YQ => format!("Y+{val}"),
            K::ZQ => format!("Z+{val}"),
            _ => String::new(),
        });
    }
    Disasm { mnemonic: def.name.into(), operands: parts.join(", "), target, words: def.words, valid: true }
}

fn fmt_target(target: u32, rel: i32, ctx: &DisasmContext) -> String {
    if let Some(label) = ctx.code_label.and_then(|f| f(target)) {
        return label;
    }
    let rel_str = if rel >= 0 { format!(".+{}", rel * 2) } else { format!(".-{}", -rel * 2) };
    format!("{rel_str} ; 0x{target:04X}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_table_rejects_low_registers_and_classic_only_ops() {
        let t = decode_table(feature::RC | feature::BREAK);
        assert_eq!(t.ops[0x0c00], 0); // add r0, r0
        assert_eq!(t.ops[0x0f00], op::ADD); // add r16, r16
        assert_eq!(t.ops[0x9601], 0); // adiw
        assert_eq!(t.ops[0xa000], op::LDS_RC);
        assert_eq!(t.ops[0x8108], op::LD_Y); // ld r16, Y (not ldd)
    }

    #[test]
    fn every_rc_instruction_round_trips() {
        let features = feature::RC | feature::BREAK;
        let t = decode_table(features);
        for def in insns().iter().filter(|d| d.is_available(features)) {
            let vals: Vec<i32> = def
                .value_operands()
                .enumerate()
                .map(|(i, k)| match k {
                    K::Rd5 | K::Rr5 | K::Rd4 | K::Rr4 => 17 + i as i32,
                    K::K7rc => 0x45,
                    K::K7 | K::K12 => -3,
                    _ => 1,
                })
                .collect();
            let words = encode(def, &vals).unwrap_or_else(|e| panic!("{}: {e}", def.name));
            let d = decode(&t, words[0], *words.get(1).unwrap_or(&0));
            assert_eq!(d.def.map(|x| x.op), Some(def.op), "{}", def.name);
            assert_eq!(d.values, vals, "{}", def.name);
        }
    }

    #[test]
    fn known_encodings() {
        let ldi = def_by_op(op::LDI).unwrap();
        assert_eq!(encode(ldi, &[16, 0xff]).unwrap(), vec![0xef0f]);
        let lds = def_by_op(op::LDS_RC).unwrap();
        assert_eq!(encode(lds, &[16, 0x40]).unwrap(), vec![0xa100]);
        let sts = def_by_op(op::STS_RC).unwrap();
        assert_eq!(encode(sts, &[0x5f, 17]).unwrap(), vec![0xab1f]);
    }

    #[test]
    fn disassembles_aliases() {
        let t = decode_table(feature::RC);
        let ctx = DisasmContext::default();
        assert_eq!(disassemble(&t, 0, 0xef0f, 0, &ctx).mnemonic, "ser");
        assert_eq!(disassemble(&t, 0, 0x2700, 0, &ctx).mnemonic, "clr");
        assert_eq!(disassemble(&t, 0, 0x9478, 0, &ctx).mnemonic, "sei");
        let br = disassemble(&t, 10, 0xf7f9, 0, &ctx);
        assert_eq!(br.mnemonic, "brne");
        assert_eq!(br.target, Some(20));
    }
}
