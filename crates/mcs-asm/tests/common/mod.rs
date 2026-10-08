//! Shared helpers for the assembler integration tests.
#![allow(dead_code)]

use std::collections::HashMap;

use mcs_asm::{assemble, assemble_with_spec, AssembleOptions, AssembleResult};
use mcs_core::avr::device::{AvrDeviceSpec, IoRegisterSpec, RegisterAccess, VectorSpec};
use mcs_core::avr::devices;
use mcs_core::avr::isa::feature;
use mcs_core::program::{Diagnostic, Severity};

pub fn asm(src: &str) -> AssembleResult {
    assemble(src, &AssembleOptions::new("main.asm", "attiny10"))
}

pub fn asm_dev(src: &str, device_id: &str) -> AssembleResult {
    assemble(src, &AssembleOptions::new("main.asm", device_id))
}

pub fn asm_inc(src: &str, includes: &[(&str, &str)]) -> AssembleResult {
    let map: HashMap<String, String> = includes.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assemble(src, &AssembleOptions::new("main.asm", "attiny10").with_includes(&map))
}

pub fn flash_words_range(r: &AssembleResult, start: usize, count: usize) -> Vec<u16> {
    let f = &r.program.flash;
    (start..start + count).map(|i| f[2 * i] as u16 | (f[2 * i + 1] as u16) << 8).collect()
}

pub fn flash_words(r: &AssembleResult) -> Vec<u16> {
    flash_words_range(r, 0, (r.program.flash_used >> 1) as usize)
}

pub fn errors(r: &AssembleResult) -> Vec<String> {
    r.diagnostics.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect()
}

/// Assembles for ATtiny10 and asserts success, returning the emitted words.
pub fn words(src: &str) -> Vec<u16> {
    let r = asm(src);
    assert_eq!(errors(&r), Vec::<String>::new(), "source: {src}");
    flash_words(&r)
}

/// First error diagnostic.
pub fn first(src: &str) -> Diagnostic {
    asm(src)
        .diagnostics
        .into_iter()
        .find(|d| d.severity == Severity::Error)
        .unwrap_or_else(|| panic!("expected an error for {src:?}"))
}

pub fn first_msg(src: &str) -> String {
    first(src).message
}

pub fn tiny10() -> &'static AvrDeviceSpec {
    devices::get("attiny10").unwrap()
}

/// Classic-core test device (cloned from the ATtiny10 description).
pub fn test_mega() -> AvrDeviceSpec {
    let mut d = tiny10().clone();
    d.id = "testmega".into();
    d.name = "TestMega".into();
    d.core_name = "AVRe+".into();
    d.features = feature::MOVW | feature::MUL | feature::LPMX | feature::JMP | feature::SPM | feature::BREAK;
    d.flash_size = 32768;
    d.sram_start = 0x100;
    d.sram_size = 2048;
    d.eeprom_size = 1024;
    d.io_base = 0x20;
    d.regs_in_data_space = true;
    d.flash_map_base = None;
    d.nvm_map = None;
    d.registers = vec![
        IoRegisterSpec {
            name: "PORTB".into(),
            addr: 0x25,
            reset: 0,
            group: "PORTB".into(),
            desc: "Port B".into(),
            bits: vec![mcs_core::avr::device::field("PORTB5", 0x20, "")],
            access: RegisterAccess::Rw,
        },
        IoRegisterSpec {
            name: "TCCR1A".into(),
            addr: 0x80,
            reset: 0,
            group: "TC1".into(),
            desc: "Timer1 control A".into(),
            bits: vec![],
            access: RegisterAccess::Rw,
        },
    ];
    d.vectors = vec![
        VectorSpec { index: 0, name: "RESET".into(), desc: "Reset".into() },
        VectorSpec { index: 1, name: "INT0".into(), desc: "External Interrupt 0".into() },
        VectorSpec { index: 16, name: "TIM0_OVF".into(), desc: "Timer0 overflow".into() },
    ];
    d
}

pub fn mega_asm(src: &str, spec: &AvrDeviceSpec) -> AssembleResult {
    assemble_with_spec(src, spec, &AssembleOptions::new("main.asm", "testmega"))
}
