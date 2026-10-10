//! RISC-V (ESP32-C3) images: ELF with `EM_RISCV` and ESP-IDF application images.
//! `data/esp32c3_uart.elf` / `data/esp32c3_calls.elf` are copies of `mcs-sim/tests/esp32c3/elf/{uart,calls}.elf`
//! (built by `mcs-sim/tests/esp32c3/gen_programs.py` with rustc for riscv32imc; `calls` carries DWARF line info).

use mcs_core::program::{ProgramFormat, Severity, SymbolKind, SymbolSpace};
use mcs_formats::{load_program_file_at, looks_like_esp_image, parse_elf_at, parse_esp_image};

const UART: &[u8] = include_bytes!("data/esp32c3_uart.elf");
const CALLS: &[u8] = include_bytes!("data/esp32c3_calls.elf");
const FLASH: usize = 4 << 20;
const IROM: u32 = 0x4200_0000;

#[test]
fn elf_segments_keep_their_run_time_addresses() {
    let p = parse_elf_at(UART, FLASH, "uart.elf", None);
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert_eq!(p.format, ProgramFormat::Elf);
    // .text in the IROM window, .rodata in the DROM window; nothing else is loadable.
    let addrs: Vec<u32> = p.segments.iter().map(|s| s.address).collect();
    assert_eq!(addrs, vec![IROM, 0x3c01_0000]);
    assert_eq!(p.segments[1].data, b"Hi\n\0");
    // The flash image (disassembly view) is the IROM segment at offset 0.
    assert_eq!((p.flash_base, p.flash.len()), (IROM, FLASH));
    assert_eq!(p.flash_used as usize, p.segments[0].data.len());
    assert_eq!(&p.flash[..p.flash_used as usize], &p.segments[0].data[..]);
    let start = p.symbols.iter().find(|s| s.name == "_start").unwrap();
    assert_eq!(p.entry, start.address);
    assert!((IROM..IROM + p.flash_used).contains(&p.entry));
}

#[test]
fn elf_symbols_are_classified_by_memory() {
    let p = parse_elf_at(UART, FLASH, "uart.elf", Some(IROM));
    let sym = |n: &str| p.symbols.iter().find(|s| s.name == n).unwrap_or_else(|| panic!("symbol {n}"));
    let start = sym("_start");
    assert_eq!((start.space, start.global), (SymbolSpace::Code, true));
    // Read-only data lives in the DROM window: a data-space symbol with an absolute address.
    let msg = sym("msg");
    assert_eq!((msg.address, msg.space, msg.kind), (0x3c01_0000, SymbolSpace::Data, SymbolKind::Label));
    assert!(p.symbols.iter().all(|s| !s.name.starts_with('$') && !s.name.starts_with(".L")), "mapping symbols are hidden");
}

#[test]
fn elf_line_table_has_absolute_addresses() {
    let p = parse_elf_at(CALLS, FLASH, "calls.elf", None);
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert!(p.files.iter().any(|f| f.ends_with("calls.rs")), "{:?}", p.files);
    assert!(!p.lines.is_empty());
    assert!(p.lines.iter().all(|l| (IROM..IROM + p.flash_used).contains(&l.address)), "{:?}", p.lines);
    let leaf = p.symbols.iter().find(|s| s.name == "leaf").unwrap();
    assert_eq!((leaf.kind, leaf.space), (SymbolKind::Func, SymbolSpace::Code));
    assert!(p.lines.iter().any(|l| l.address == leaf.address && l.is_stmt));
}

#[test]
fn elf_larger_than_the_flash_is_reported() {
    let p = parse_elf_at(UART, 16, "uart.elf", Some(IROM));
    assert!(p.has_errors());
    assert!(p.diagnostics.iter().any(|d| d.severity == Severity::Error && d.message.contains("beyond the end of flash")), "{:?}", p.diagnostics);
    // The run-time segments are still complete.
    assert_eq!(p.segments.len(), 2);
}

fn image(segments: &[(u32, &[u8])], entry: u32, chip: u16) -> Vec<u8> {
    let mut b = vec![0xe9, segments.len() as u8, 2, 0x20];
    b.extend_from_slice(&entry.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]); // wp_pin, spi_pin_drv[3]
    b.extend_from_slice(&chip.to_le_bytes());
    b.extend_from_slice(&[0; 10]); // min_chip_rev .. hash_appended
    assert_eq!(b.len(), 24);
    for (addr, data) in segments {
        b.extend_from_slice(&addr.to_le_bytes());
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(data);
    }
    b.extend_from_slice(&[0; 16]); // checksum byte + padding
    b
}

#[test]
fn esp_application_image_places_its_segments() {
    let code = [0x13u8, 0, 0, 0, 0x73, 0, 0x10, 0]; // nop; ebreak
    let data = [1u8, 2, 3, 4];
    let bin = image(&[(IROM + 0x20, &code), (0x3fc8_0000, &data), (0x3c00_0020, b"rodata\0\0")], IROM + 0x20, 5);
    assert!(looks_like_esp_image(&bin));
    let p = load_program_file_at(&bin, "app.bin", FLASH, IROM);
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert_eq!(p.entry, IROM + 0x20);
    assert_eq!(p.segments.iter().map(|s| s.address).collect::<Vec<_>>(), vec![IROM + 0x20, 0x3fc8_0000, 0x3c00_0020]);
    assert_eq!((p.flash_base, p.flash_used), (IROM, 0x28));
    assert_eq!(&p.flash[0x20..0x28], &code);
    assert_eq!(p.flash[0], 0xff, "gaps stay erased");
    // A different chip id only warns.
    let other = parse_esp_image(&image(&[(IROM, &code)], IROM, 9), FLASH, "app.bin", None);
    assert!(!other.has_errors() && other.diagnostics.iter().any(|d| d.severity == Severity::Warning && d.message.contains("chip id")), "{:?}", other.diagnostics);
}

#[test]
fn malformed_esp_images_are_reported_not_panics() {
    let good = image(&[(IROM, &[0u8; 16])], IROM, 5);
    // Truncated segment data.
    let cut = &good[..24 + 8 + 4];
    let p = parse_esp_image(cut, FLASH, "app.bin", None);
    assert!(p.has_errors() && p.diagnostics.iter().any(|d| d.message.contains("beyond the end")), "{:?}", p.diagnostics);
    // Truncated segment header.
    let p = parse_esp_image(&good[..28], FLASH, "app.bin", None);
    assert!(p.has_errors());
    // Wrong magic: not an image at all.
    let mut bad = good.clone();
    bad[0] = 0;
    assert!(!looks_like_esp_image(&bad));
    assert!(parse_esp_image(&bad, FLASH, "app.bin", None).has_errors());
    // A segment that wraps the address space.
    let wrap = image(&[(0xffff_fff0, &[0u8; 32])], IROM, 5);
    assert!(parse_esp_image(&wrap, FLASH, "app.bin", None).has_errors());
}
