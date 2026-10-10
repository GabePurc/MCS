//! Host-independent services behind the UI: building assembly sources, importing program
//! images, disassembly and device information. The Tauri app and the WebAssembly build are
//! thin adapters over these functions, so both front-ends behave identically.

use std::collections::HashMap;

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::devices;
pub use mcs_core::avr::devices::CustomMcuConfig;
use mcs_core::device::{Arch, DeviceRef};
use mcs_core::avr::isa::{self, DisasmContext};
use mcs_core::avr::{isa_docs, isa_usage};
use mcs_core::program::{Diagnostic, LoadedProgram};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub arch: Arch,
    pub id: String,
    pub name: String,
    pub family: String,
    pub flash_size: u32,
    pub sram_size: u32,
    pub package: String,
    pub core_name: String,
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
    /// Word address (AVR) or byte address (ARM).
    pub pc: u32,
    /// Instruction length in 16-bit words.
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
    pub summary: String,
    pub operation: String,
    pub flags: String,
    pub aliases: String,
    /// Beginner help: what the instruction is for and how to use it.
    pub usage: String,
    pub example: String,
    /// Canonical mnemonic for an assembler alias row ('' for real instructions).
    pub alias_of: String,
}

/// The AVR spec for `device_id`; None for unknown ids and for other architectures (the
/// disassembler, instruction set and definition files are AVR-only for now).
fn avr_spec(device_id: &str) -> Option<&'static AvrDeviceSpec> {
    mcs_core::devices::get_any(device_id)?.as_avr()
}

pub fn list_devices() -> Vec<DeviceSummary> {
    mcs_core::devices::list_any().into_iter().map(summary).collect()
}

fn summary(d: DeviceRef) -> DeviceSummary {
    match d {
        DeviceRef::Avr(s) => avr_summary(s),
        DeviceRef::Arm(s) => DeviceSummary {
            arch: Arch::Arm,
            id: s.id.clone(),
            name: s.name.clone(),
            family: s.family.clone(),
            flash_size: s.flash_size,
            sram_size: s.ram_total(),
            package: s.package.clone(),
            core_name: s.core_name.clone(),
        },
    }
}

fn avr_summary(d: &AvrDeviceSpec) -> DeviceSummary {
    DeviceSummary {
        arch: Arch::Avr,
        id: d.id.clone(),
        name: d.name.clone(),
        family: d.family.clone(),
        flash_size: d.flash_size,
        sram_size: d.sram_size as u32,
        package: d.package.clone(),
        core_name: d.core_name.clone(),
    }
}

/// Outcome of registering one custom device (`error` is None on success).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomRegistration {
    pub arch: Arch,
    pub id: String,
    pub error: Option<String>,
}

/// Registers (or replaces) user-defined devices in this process.
pub fn register_custom_devices(configs: &[CustomMcuConfig]) -> Vec<CustomRegistration> {
    configs.iter().map(|c| CustomRegistration { arch: Arch::Avr, id: c.id.clone(), error: devices::register_custom(c).err() }).collect()
}

/// What a custom configuration turns into (for the editor's live summary).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPreview {
    pub package: String,
    pub pins: usize,
    pub gpios: u8,
    pub registers: usize,
    pub vectors: usize,
    pub core_name: String,
    pub sram_start: u16,
    pub ram_end: u16,
    /// Peripheral groups ("TC1", "USART2", ...).
    pub groups: Vec<String>,
}

pub fn custom_device_preview(config: &CustomMcuConfig) -> Result<CustomPreview, String> {
    let d = config.build()?;
    Ok(CustomPreview {
        package: d.package.clone(),
        pins: d.pins.len(),
        gpios: d.gpio_count,
        registers: d.registers.len(),
        vectors: d.vector_count(),
        core_name: d.core_name.clone(),
        sram_start: d.sram_start,
        ram_end: d.ram_end(),
        groups: d.groups.iter().map(|g| g.name.clone()).collect(),
    })
}

pub fn custom_device_defaults() -> CustomMcuConfig {
    CustomMcuConfig::default()
}

/// Assembles a source with the built-in assembler. `includes` maps `.include` names to text.
pub fn build_asm(source: &str, file_name: &str, device_id: &str, includes: &HashMap<String, String>) -> BuildOutcome {
    let r = mcs_asm::assemble(source, &mcs_asm::AssembleOptions { file_name, device_id, includes });
    BuildOutcome { ok: r.ok, program: Some(r.program), diagnostics: r.diagnostics, output: String::new(), listing: Some(r.listing), device_id: r.device_id }
}

/// Parses an ELF or Intel HEX image. The device embedded in an ELF wins when known.
pub fn import_program(bytes: &[u8], file_name: &str, device_id: &str) -> BuildOutcome {
    let dev = mcs_core::devices::get_any(device_id);
    let flash = dev.map(|s| s.flash_size() as usize).unwrap_or(1024);
    let program = mcs_formats::load_program_file_at(bytes, file_name, flash, dev.map_or(0, |d| d.flash_base()));
    let device = program.device.clone().filter(|d| mcs_core::devices::get_any(d).is_some()).unwrap_or_else(|| device_id.to_string());
    BuildOutcome { ok: !program.has_errors(), diagnostics: program.diagnostics.clone(), program: Some(program), output: String::new(), listing: None, device_id: device }
}

/// Parses the ELF produced by an external compiler (C builds).
pub fn program_from_elf(elf: &[u8], file_name: &str, device_id: &str, extra: Vec<Diagnostic>, output: String) -> BuildOutcome {
    let dev = mcs_core::devices::get_any(device_id);
    let flash = dev.map(|s| s.flash_size() as usize).unwrap_or(1024);
    let mut program = mcs_formats::parse_elf_at(elf, flash, file_name, dev.map(|d| d.flash_base()).filter(|&b| b != 0));
    program.diagnostics.extend(extra);
    BuildOutcome { ok: !program.has_errors(), diagnostics: program.diagnostics.clone(), program: Some(program), output, listing: None, device_id: device_id.to_string() }
}

/// Builds a machine-code (`.mc`) source.
pub fn build_machine_code(source: &str, file_name: &str, device_id: &str) -> BuildOutcome {
    let r = mcs_asm::assemble_machine_code(source, file_name, device_id);
    BuildOutcome { ok: r.ok, program: Some(r.program), diagnostics: r.diagnostics, output: String::new(), listing: Some(r.listing), device_id: r.device_id }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McAnnotations {
    pub hints: Vec<mcs_asm::McHint>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Live editor feedback for machine-code sources: per-line disassembly and problems.
pub fn machine_code_hints(source: &str, device_id: &str) -> McAnnotations {
    let r = mcs_asm::assemble_machine_code(source, "", device_id);
    McAnnotations { hints: r.hints, diagnostics: r.diagnostics }
}

/// Renders a program image as an editable machine-code source: one instruction per line with
/// its disassembly as a comment, labels as comment lines.
pub fn program_to_machine_code(device_id: &str, flash: &[u8], used: usize, labels: &HashMap<u32, String>, title: &str) -> String {
    if mcs_core::devices::get_any(device_id).is_some_and(|d| d.as_avr().is_none()) {
        return "; Machine-code sources are available for AVR devices only.\n".to_string();
    }
    let name = mcs_core::devices::get_any(device_id).map(|d| d.name()).unwrap_or(device_id);
    let mut out = format!(
        "; Machine code for {name}{}\n; One instruction per line as 16-bit words in hex (two words for 32-bit instructions).\n; Edit the words and build (F7) to run them. '@0x0010' moves to a byte address.\n\n@0x0000\n",
        if title.is_empty() { String::new() } else { format!(" - generated from {title}") },
    );
    let used = used.min(flash.len()).div_ceil(2) * 2;
    let mut skipping = false;
    for l in disassemble(device_id, &flash[..used], labels) {
        let addr = l.pc * 2;
        // Unprogrammed gaps (0xFFFF) are skipped with an address directive.
        if l.raw.iter().all(|&w| w == 0xffff) {
            skipping = true;
            continue;
        }
        if skipping {
            out.push_str(&format!("\n@0x{addr:04X}\n"));
            skipping = false;
        }
        if let Some(label) = labels.get(&addr) {
            out.push_str(&format!("; {label}:\n"));
        }
        let words = l.raw.iter().map(|w| format!("{w:04X}")).collect::<Vec<_>>().join(" ");
        let asm = if l.operands.is_empty() { l.mnemonic.clone() } else { format!("{} {}", l.mnemonic, l.operands) };
        out.push_str(&format!("{words:<12}; {addr:04X}  {asm}\n"));
    }
    out
}

/// Disassembles a whole program image. `labels` maps code byte addresses to names.
pub fn disassemble(device_id: &str, flash: &[u8], labels: &HashMap<u32, String>) -> Vec<DisasmLine> {
    if let Some(arm) = mcs_core::devices::get_any(device_id).and_then(|d| d.as_arm()) {
        return disassemble_arm(arm, flash, labels);
    }
    let Some(spec) = avr_spec(device_id) else { return Vec::new() };
    let table = isa::decode_table(spec.features);
    let io_names: HashMap<u32, String> = spec.registers.iter().rev().filter_map(|r| spec.data_to_io(r.addr).map(|io| (io as u32, r.name.clone()))).collect();
    let data_names: HashMap<u32, String> = spec.registers.iter().rev().map(|r| (r.addr as u32, r.name.clone())).collect();
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

/// ARM disassembly: `pc` is the absolute byte address (flash base included), `raw` holds the one or
/// two Thumb halfwords, `words` counts them. Branch targets found in the operands are resolved
/// through `labels` (code byte addresses).
fn disassemble_arm(spec: &mcs_core::arm::device::ArmDeviceSpec, flash: &[u8], labels: &HashMap<u32, String>) -> Vec<DisasmLine> {
    let features = mcs_core::arm::thumb::ArmFeatures(spec.features);
    let len = flash.len() & !1;
    mcs_core::arm::disasm::disassemble(&flash[..len], spec.flash_base, features)
        .into_iter()
        .map(|(addr, n, text)| {
            let off = (addr - spec.flash_base) as usize;
            let raw: Vec<u16> = (0..n / 2).map(|k| u16::from_le_bytes([flash[off + 2 * k], flash[off + 2 * k + 1]])).collect();
            let (mnemonic, mut operands) = match text.split_once([' ', '\t']) {
                Some((m, o)) => (m.to_string(), o.trim().to_string()),
                None => (text.clone(), String::new()),
            };
            let valid = !text.starts_with("<undefined>");
            // Direct branch targets are printed as 0x<hex>.
            let is_branch = mnemonic.starts_with('b') && !mnemonic.starts_with("bf") && !mnemonic.starts_with("bic") && !mnemonic.starts_with("bkpt") && !mnemonic.starts_with("bx") && !mnemonic.starts_with("blx") || mnemonic.starts_with("cb");
            let target = if is_branch {
                operands.rsplit([' ', ',']).next().and_then(|t| t.strip_prefix("0x")).and_then(|h| u32::from_str_radix(h, 16).ok())
            } else {
                None
            };
            if let Some(name) = target.and_then(|t| labels.get(&t)) {
                if let Some(i) = operands.rfind("0x") {
                    operands.replace_range(i.., name);
                }
            }
            DisasmLine { pc: addr, words: (n / 2) as u8, raw, mnemonic, operands, target, valid }
        })
        .collect()
}

pub fn instruction_set(device_id: &str) -> Vec<InsnInfo> {
    let Some(spec) = avr_spec(device_id) else { return Vec::new() };
    let rc = spec.features & isa::feature::RC != 0;
    let mut out: Vec<InsnInfo> = isa::insns()
        .iter()
        .filter(|d| d.is_available(spec.features))
        .map(|d| {
            let doc = isa_docs::insn_doc(d.name);
            let usage = isa_usage::insn_usage(d.name);
            InsnInfo {
                mnemonic: d.name.to_uppercase(),
                operands: d.operands.iter().map(|k| operand_label(*k)).collect::<Vec<_>>().join(", "),
                encoding: d.pattern.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" "),
                cycles: if rc { d.cycles_rc } else { d.cycles },
                words: d.words,
                summary: doc.map(|x| x.summary).unwrap_or_default().into(),
                operation: doc.map(|x| x.operation).unwrap_or_default().into(),
                flags: doc.map(|x| x.flags).unwrap_or_default().into(),
                aliases: doc.map(|x| x.aliases).unwrap_or_default().into(),
                usage: usage.map(|x| x.help).unwrap_or_default().into(),
                example: usage.map(|x| x.example).unwrap_or_default().into(),
                alias_of: String::new(),
            }
        })
        .collect();
    // Aliases (BRNE, CLR, ...) of the instructions this device has, after the real ones.
    for a in isa_usage::ALIASES {
        let Some(base) = out.iter().find(|i| i.alias_of.is_empty() && i.mnemonic.eq_ignore_ascii_case(a.of)) else { continue };
        let row = InsnInfo {
            mnemonic: a.name.to_uppercase(),
            operands: a.operands.into(),
            encoding: base.encoding.clone(),
            cycles: base.cycles,
            words: base.words,
            summary: a.summary.into(),
            operation: a.operation.into(),
            flags: base.flags.clone(),
            aliases: String::new(),
            usage: a.usage.help.into(),
            example: a.usage.example.into(),
            alias_of: base.mnemonic.clone(),
        };
        out.push(row);
    }
    out
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
    avr_spec(device_id).map(|s| (mcs_asm::def_include_name(s), mcs_asm::generate_def_include(s)))
}

/// `.include` names referenced by a source that are not built-in device definitions.
pub fn user_includes(source: &str) -> Vec<String> {
    mcs_asm::scan_includes(source).into_iter().filter(|n| devices::id_from_include_name(n).is_none()).collect()
}

/// Intel HEX text for the first `used` bytes of a program image.
pub fn to_intel_hex(flash: &[u8], used: usize) -> String {
    mcs_formats::to_intel_hex(flash, 0, used.min(flash.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLINK_ELF: &[u8] = include_bytes!("../../mcs-formats/tests/data/stm32g4_blink.elf");

    #[test]
    fn arm_devices_listing_import_and_disassembly() {
        let devs = list_devices();
        let g4 = devs.iter().find(|d| d.id == "stm32g474re").expect("STM32G474RE is listed");
        assert_eq!((g4.arch, g4.flash_size, g4.sram_size, g4.core_name.as_str(), g4.package.as_str()), (Arch::Arm, 512 * 1024, 128 * 1024, "Cortex-M4F", "LQFP64"));
        assert!(devs.iter().any(|d| d.id == "stm32g431kb" && d.arch == Arch::Arm));
        assert!(devs.iter().any(|d| d.id == "attiny10" && d.arch == Arch::Avr));

        // The AVR-only assemblers refuse ARM devices with a clear message.
        let a = build_asm("nop", "t.s", "stm32g474re", &HashMap::new());
        assert!(!a.ok && a.diagnostics[0].message.contains("AVR devices only"), "{:?}", a.diagnostics);
        let m = build_machine_code("0000", "t.mc", "stm32g431kb");
        assert!(!m.ok && m.diagnostics[0].message.contains("ARM Cortex-M"), "{:?}", m.diagnostics);

        // ELF import places the image relative to the flash base; symbols stay absolute.
        let r = import_program(BLINK_ELF, "blink.elf", "stm32g474re");
        assert!(r.ok, "{:?}", r.diagnostics);
        let p = r.program.unwrap();
        assert_eq!((p.flash_base, p.flash.len(), p.entry), (0x0800_0000, 512 * 1024, 0x0800_00e8));

        // Disassembly: byte addresses, Thumb halfwords, labels substituted in branch targets.
        let delay = p.symbols.iter().find(|s| s.name == "delay").unwrap().address;
        let labels = HashMap::from([(delay, "delay".to_string())]);
        let lines = disassemble("stm32g474re", &p.flash[..p.flash_used as usize], &labels);
        let reset = lines.iter().find(|l| l.pc == 0x0800_00e8).expect("reset handler");
        assert_eq!((reset.mnemonic.as_str(), reset.operands.as_str(), reset.words, reset.raw.len()), ("movw", "r0, #0x1000", 2, 2));
        let bl = lines.iter().find(|l| l.mnemonic == "bl").expect("a call");
        assert_eq!((bl.operands.as_str(), bl.target), ("delay", Some(delay)));
        assert!(lines.iter().all(|l| l.valid || l.pc < 0x0800_00e8), "code after the vector table decodes");
        // Machine-code export is AVR-only.
        assert!(program_to_machine_code("stm32g474re", &p.flash, 16, &HashMap::new(), "x").contains("AVR devices only"));
    }

    #[test]
    fn machine_code_round_trip() {
        let asm = build_asm("ldi r16, 1\nloop: rjmp loop\n.org 0x10\nnop\n", "t.asm", "attiny10", &HashMap::new());
        let p = asm.program.unwrap();
        let labels = HashMap::from([(2u32, "loop".to_string())]);
        let mc = program_to_machine_code("attiny10", &p.flash, p.flash_used as usize, &labels, "t.asm");
        assert!(mc.contains("E001        ; 0000  ldi r16, 0x01"), "{mc}");
        assert!(mc.contains("; loop:\nCFFF"), "{mc}");
        assert!(mc.contains("@0x0020\n0000"), "{mc}");
        let back = build_machine_code(&mc, "t.mc", "attiny10");
        assert!(back.ok, "{:?}", back.diagnostics);
        assert_eq!(back.program.unwrap().flash, p.flash);
        let h = machine_code_hints("E001\nZZ", "attiny10");
        assert_eq!(h.hints[0].text, "ldi r16, 0x01");
        assert_eq!(h.diagnostics.len(), 1);
    }
}
