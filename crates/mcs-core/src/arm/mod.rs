//! ARM architecture definitions (ARMv7-M Thumb/Thumb-2 instruction set).
//!
//! * [`thumb`] — instruction decoder: encodings -> [`thumb::Insn`] (shared by the executor and the
//!   disassembler).
//! * [`disasm`] — UAL disassembler matching `llvm-objdump --triple=thumbv7em`.
//!
//! References: ARM DDI 0403E.e (ARMv7-M Architecture Reference Manual), chapters A5/A7.

pub mod disasm;
pub mod thumb;
