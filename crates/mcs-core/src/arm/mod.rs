//! ARM architecture definitions (ARMv7-M Thumb/Thumb-2 instruction set, device descriptions).
//!
//! * [`thumb`] — instruction decoder: encodings -> [`thumb::Insn`] (shared by the executor and the
//!   disassembler).
//! * [`disasm`] — UAL disassembler matching `llvm-objdump --triple=thumbv7em`.
//! * [`device`] / [`devices`] — declarative device descriptions (STM32G4) and their registry.
//!
//! References: ARM DDI 0403E.e (ARMv7-M Architecture Reference Manual), chapters A5/A7.

pub mod device;
pub mod devices;
pub mod disasm;
pub mod thumb;
