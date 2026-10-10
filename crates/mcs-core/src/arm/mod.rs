//! ARM architecture definitions (ARMv7-M Thumb/Thumb-2 instruction set, device descriptions).
//!
//! * [`thumb`] — instruction decoder: encodings -> [`thumb::Insn`] (shared by the executor and the
//!   disassembler).
//! * [`vfp`] — floating-point (FPv4-SP / FPv5-D16) encodings and operand conventions.
//! * [`disasm`] — UAL disassembler matching `llvm-objdump --triple=thumbv7em`.
//! * [`device`] / [`devices`] — declarative device descriptions (STM32G4) and their registry.
//!
//! References: ARM DDI 0403E.e (ARMv7-M Architecture Reference Manual), chapters A5/A6/A7.

pub mod device;
pub mod devices;
pub mod disasm;
mod disasm_ext;
pub mod thumb;
pub mod vfp;
