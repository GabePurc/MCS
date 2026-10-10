//! RISC-V definitions: the RV32IMC + Zicsr + Zifencei instruction set (used by the ESP32-C3).
//!
//! * [`mod@decode`] — instruction decoder: encodings -> [`decode::Insn`] (shared by the executor and the
//!   disassembler); compressed instructions are expanded to their base form.
//! * [`disasm`] — disassembler matching `llvm-objdump` for `riscv32`.
//! * [`device`] / [`devices`] — declarative device descriptions (ESP32-C3) and their registry.
//! * [`csr_names`] — standard CSR names (generated from `llvm-objdump`).
//!
//! References: The RISC-V Instruction Set Manual, Volume I (Unprivileged ISA, 20240411) and
//! Volume II (Privileged Architecture, 20240411).

pub mod csr_names;
pub mod decode;
pub mod device;
pub mod devices;
pub mod disasm;

pub use decode::{decode, insn_len, Insn, Op};
pub use disasm::{disassemble, format_insn};
