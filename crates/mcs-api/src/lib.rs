//! Host-independent services behind the UI: building assembly sources, importing program
//! images, disassembly and device information. The Tauri app and the WebAssembly build are
//! thin adapters over these functions, so both front-ends behave identically.

use std::collections::HashMap;

use mcs_core::avr::devices;
use mcs_core::avr::isa::{self, DisasmContext};
use mcs_core::program::{Diagnostic, LoadedProgram};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub family: String,
    pub flash_size: u32,
    pub sram_size: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOutcome {
    pub ok: bool,
    pub program: Option<LoadedProgram>,
    pub diagnostics: Vec<Diagnostic>,
    pub output: String,
    pub listing: Option<String>,
    pub device_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisasmLine {
    /// Word address.
    pub pc: u32,
    pub words: u8,
    pub raw: Vec<u16>,
    pub mnemonic: String,
    pub operands: String,
    pub target: Option<u32>,
    pub valid: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsnInfo {
    pub mnemonic: String,
    pub operands: String,
    pub encoding: String,
    pub cycles: u8,
    pub words: u8,
}

pub fn list_devices() -> Vec<DeviceSummary> {
    devices::all()
        .iter()
        .map(|d| DeviceSummary { id: d.id.clone(), name: d.name.clone(), family: d.family.clone(), flash_size: d.flash_size, sram_size: d.sram_size })
        .collect()
}

/// Assembles a source with the built-in assembler. `includes` maps `.include` names to text.
pub fn build_asm(source: &str, file_name: &str, device_id: &str, includes: &HashMap<String, String>) -> BuildOutcome {
    let r = mcs_asm::assemble(source, &mcs_asm::AssembleOptions { file_name, device_id, includes });
    BuildOutcome { ok: r.ok, program: Some(r.program), diagnostics: r.diagnostics, output: String::new(), listing: Some(r.listing), device_id: r.device_id }
}

/// Parses an ELF or Intel HEX image. The device embedded in an ELF wins when known.
pub fn import_program(bytes: &[u8], file_name: &str, device_id: &str) -> BuildOutcome {
    let flash = devices::get(device_id).map(|s| s.flash_size as usize).unwrap_or(1024);
    let program = mcs_formats::load_program_file(bytes, file_name, flash);
    let device = program.device.clone().filter(|d| devices::get(d).is_some()).unwrap_or_else(|| device_id.to_string());
    BuildOutcome { ok: !program.has_errors(), diagnostics: program.diagnostics.clone(), program: Some(program), output: String::new(), listing: None, device_id: device }
}

/// Parses the ELF produced by an external compiler (C builds).
pub fn program_from_elf(elf: &[u8], file_name: &str, device_id: &str, extra: Vec<Diagnostic>, output: String) -> BuildOutcome {
    let flash = devices::get(device_id).map(|s| s.flash_size as usize).unwrap_or(1024);
    let mut program = mcs_formats::parse_elf(elf, flash, file_name);
    program.diagnostics.extend(extra);
    BuildOutcome { ok: !program.has_errors(), diagnostics: program.diagnostics.clone(), program: Some(program), output, listing: None, device_id: device_id.to_string() }
}

/// Disassembles a whole program image. `labels` maps code byte addresses to names.
pub fn disassemble(device_id: &str, flash: &[u8], labels: &HashMap<u32, String>) -> Vec<DisasmLine> {
    let Some(spec) = devices::get(device_id) else { return Vec::new() };
    let table = isa::decode_table(spec.features);
    let io_names: HashMap<u32, String> = spec.registers.iter().filter_map(|r| spec.data_to_io(r.addr).map(|io| (io as u32, r.name.clone()))).collect();
    let data_names: HashMap<u32, String> = spec.registers.iter().map(|r| (r.addr as u32, r.name.clone())).collect();
    let code_label = |a: u32| labels.get(&a).cloned();
    let io_name = |a: u32| io_names.get(&a).cloned();
    let data_name = |a: u32| data_names.get(&a).cloned();
    let ctx = DisasmContext { code_label: Some(&code_label), io_name: Some(&io_name), data_name: Some(&data_name), flash_bytes: spec.flash_size };
    let words = (flash.len() / 2) as u32;
    let word = |i: u32| if i < words { u16::from_le_bytes([flash[i as usize * 2], flash[i as usize * 2 + 1]]) } else { 0xffff };
    let mut out = Vec::with_capacity(words as usize);
    let mut pc = 0;
    while pc < words {
        let d = isa::disassemble(&table, pc, word(pc), word(pc + 1), &ctx);
        let n = d.words.max(1) as u32;
        out.push(DisasmLine { pc, words: n as u8, raw: (0..n).map(|k| word(pc + k)).collect(), mnemonic: d.mnemonic, operands: d.operands, target: d.target, valid: d.valid });
        pc += n;
    }
    out
}

pub fn instruction_set(device_id: &str) -> Vec<InsnInfo> {
    let Some(spec) = devices::get(device_id) else { return Vec::new() };
    let rc = spec.features & isa::feature::RC != 0;
    isa::insns()
        .iter()
        .filter(|d| d.is_available(spec.features))
        .map(|d| InsnInfo {
            mnemonic: d.name.to_uppercase(),
            operands: d.operands.iter().map(|k| operand_label(*k)).collect::<Vec<_>>().join(", "),
            encoding: d.pattern.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" "),
            cycles: if rc { d.cycles_rc } else { d.cycles },
            words: d.words,
        })
        .collect()
}

fn operand_label(k: isa::OperandKind) -> String {
    use isa::OperandKind as K;
    match k {
        K::Rd5 | K::Rd4 | K::Rd3 | K::RdW | K::RdP => "Rd".into(),
        K::Rr5 | K::Rr4 | K::Rr3 | K::RrW => "Rr".into(),
        K::K8 | K::K6 | K::K4 => "K".into(),
        K::A5 | K::A6 => "A".into(),
        K::B => "b".into(),
        K::S => "s".into(),
        K::K7 | K::K12 | K::K22 | K::K16 | K::K7rc => "k".into(),
        other => other.literal_text().into(),
    }
}

/// (include file name, generated avrasm2 definitions) for a device.
pub fn def_include(device_id: &str) -> Option<(String, String)> {
    devices::get(device_id).map(|s| (mcs_asm::def_include_name(s), mcs_asm::generate_def_include(s)))
}

/// `.include` names referenced by a source that are not built-in device definitions.
pub fn user_includes(source: &str) -> Vec<String> {
    mcs_asm::scan_includes(source).into_iter().filter(|n| devices::id_from_include_name(n).is_none()).collect()
}

/// Intel HEX text for the first `used` bytes of a program image.
pub fn to_intel_hex(flash: &[u8], used: usize) -> String {
    mcs_formats::to_intel_hex(flash, 0, used.min(flash.len()))
}
