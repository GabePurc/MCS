//! ARM (Cortex-M) images: ELF with `EM_ARM` and Intel HEX at the STM32 flash base 0x0800_0000.
//! `data/stm32g4_blink.elf` is built from `mcs-sim/tests/stm32g4/programs/blink.s` by
//! `mcs-sim/tests/stm32g4/make_elf.py` (Apple clang + a hand-written ELF layout).

use mcs_core::program::{ProgramFormat, Severity, SymbolKind, SymbolSpace};
use mcs_formats::{load_program_file_at, parse_elf_at, parse_intel_hex_at};

const ELF: &[u8] = include_bytes!("data/stm32g4_blink.elf");
const BASE: u32 = 0x0800_0000;

#[test]
fn elf_segments_load_into_flash_relative_to_the_base() {
    let p = parse_elf_at(ELF, 512 * 1024, "blink.elf", Some(BASE));
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert_eq!(p.format, ProgramFormat::Elf);
    assert_eq!(p.flash.len(), 512 * 1024);
    assert_eq!(p.flash_base, BASE);
    assert_eq!(p.flash_used, 0x12c);
    // Vector table: initial SP, then the reset handler with the Thumb bit.
    assert_eq!(&p.flash[0..4], &0x2002_0000u32.to_le_bytes());
    assert_eq!(&p.flash[4..8], &0x0800_00e9u32.to_le_bytes());
    // The entry point has the Thumb bit cleared.
    assert_eq!(p.entry, 0x0800_00e8);
    // The default base is the STM32 flash for EM_ARM images.
    let q = parse_elf_at(ELF, 512 * 1024, "blink.elf", None);
    assert_eq!((q.flash_base, q.flash_used), (BASE, 0x12c));
}

#[test]
fn elf_symbols_have_the_thumb_bit_cleared() {
    let p = parse_elf_at(ELF, 512 * 1024, "blink.elf", Some(BASE));
    let sym = |n: &str| p.symbols.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("symbol {n}"));
    let reset = sym("reset");
    assert_eq!((reset.address, reset.kind, reset.space, reset.global), (0x0800_00e8, SymbolKind::Func, SymbolSpace::Code, true));
    assert_eq!(sym("blink_loop").address, 0x0800_0108, "label in an executable section");
    let delay = sym("delay");
    assert_eq!((delay.address & 1, delay.global), (0, false));
    assert!(p.symbols.windows(2).all(|w| w[0].address <= w[1].address || w[0].space != w[1].space));
}

#[test]
fn elf_line_table_uses_absolute_addresses() {
    let p = parse_elf_at(ELF, 512 * 1024, "blink.elf", Some(BASE));
    assert!(p.files.iter().any(|f| f.ends_with("blink.s")), "{:?}", p.files);
    assert!(!p.lines.is_empty());
    assert!(p.lines.iter().all(|l| (BASE..BASE + 0x12c).contains(&l.address)), "{:?}", p.lines);
    // A statement starts at the loop label.
    assert!(p.lines.iter().any(|l| l.address == 0x0800_0108 && l.is_stmt));
}

#[test]
fn elf_larger_than_the_flash_is_reported() {
    let p = parse_elf_at(ELF, 128, "blink.elf", Some(BASE));
    assert!(p.has_errors());
    assert!(p.diagnostics.iter().any(|d| d.severity == Severity::Error && d.message.contains("beyond the end of flash")), "{:?}", p.diagnostics);
}

fn hex_record(addr: u16, kind: u8, data: &[u8]) -> String {
    let mut bytes = vec![data.len() as u8, (addr >> 8) as u8, addr as u8, kind];
    bytes.extend_from_slice(data);
    let sum = bytes.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    bytes.push(0u8.wrapping_sub(sum));
    format!(":{}\n", bytes.iter().map(|b| format!("{b:02X}")).collect::<String>())
}

#[test]
fn intel_hex_with_extended_linear_address_records() {
    let mut text = String::new();
    text += &hex_record(0, 4, &[0x08, 0x00]); // ELA: 0x0800_0000
    text += &hex_record(0, 0, &[1, 2, 3, 4]);
    text += &hex_record(0x0100, 0, &[0xaa, 0xbb]);
    text += &hex_record(0, 5, &[0x08, 0x00, 0x00, 0xe9]); // start linear address
    text += &hex_record(0, 1, &[]);
    let p = parse_intel_hex_at(&text, 1024, "a.hex", BASE);
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert_eq!(p.flash_base, BASE);
    assert_eq!(&p.flash[..4], &[1, 2, 3, 4]);
    assert_eq!(&p.flash[0x100..0x102], &[0xaa, 0xbb]);
    assert_eq!(p.flash_used, 0x102);
    assert_eq!(p.entry, 0x0800_00e9);
    // Without the base the same file overflows an AVR-sized flash.
    let q = parse_intel_hex_at(&text, 1024, "a.hex", 0);
    assert!(q.has_errors());
    // The generic entry point picks the loader from the content and applies the base.
    let r = load_program_file_at(text.as_bytes(), "a.hex", 1024, BASE);
    assert_eq!(&r.flash[..4], &[1, 2, 3, 4]);
    let e = load_program_file_at(ELF, "x.bin", 512 * 1024, BASE);
    assert_eq!(e.format, ProgramFormat::Elf);
}
