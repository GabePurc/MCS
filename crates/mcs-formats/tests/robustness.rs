//! Fuzz-style robustness tests: truncated and corrupted versions of the fixtures must produce
//! diagnostics, never panics (any panic fails the test).

mod common;

use common::*;
use mcs_formats::dwarf::{parse_debug_line, DebugLineOptions};
use mcs_formats::{load_program_file, parse_elf, parse_intel_hex, to_intel_hex};

fn elf_inputs() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("dwarf4", fixture("line_fixture.dwarf4.o")),
        ("dwarf5", fixture("line_fixture.dwarf5.o")),
        ("avr", build_avr_elf(&build_debug_line())),
        ("be64", build_be64_elf()),
    ]
}

/// Loader invariants that must hold for any input.
fn check_elf(bytes: &[u8], flash_size: usize) {
    let p = parse_elf(bytes, flash_size, "fuzz.elf");
    assert_eq!(p.flash.len(), flash_size);
    assert!(p.flash_used as usize <= flash_size);
    assert!(p.lines.windows(2).all(|w| w[0].address <= w[1].address));
    assert!(p.lines.iter().all(|l| l.line > 0 && (l.file as usize) < p.files.len()));
    assert!(p.diagnostics.iter().all(|d| d.file == "fuzz.elf"));
}

#[test]
fn truncated_elf_files_never_panic() {
    for (name, bytes) in elf_inputs() {
        for len in 0..bytes.len() {
            let p = parse_elf(&bytes[..len], 0x1000, name);
            // Cutting a file short always loses something the headers promise.
            assert!(!p.diagnostics.is_empty(), "{name}: no diagnostic for a {len}-byte prefix");
            check_elf(&bytes[..len], 0x1000);
        }
    }
}

#[test]
fn corrupted_elf_files_never_panic() {
    for (_, bytes) in elf_inputs() {
        let mut copy = bytes.clone();
        for i in 0..bytes.len() {
            for value in [0x00, 0xff, 0x80, 0x7f, bytes[i] ^ 0x01, bytes[i].wrapping_add(0x10)] {
                copy[i] = value;
                check_elf(&copy, 0x1000);
            }
            copy[i] = bytes[i];
        }
    }
}

#[test]
fn randomly_mutated_elf_files_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for (_, bytes) in elf_inputs() {
        for _ in 0..3000 {
            let mut copy = bytes.clone();
            for _ in 0..1 + rng.below(8) {
                let at = rng.below(copy.len());
                copy[at] = rng.next() as u8;
            }
            if rng.below(4) == 0 {
                copy.truncate(rng.below(copy.len() + 1));
            }
            check_elf(&copy, 1 + rng.below(0x200));
        }
    }
}

fn debug_line_inputs() -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let d4 = fixture("line_fixture.dwarf4.o");
    let d5 = fixture("line_fixture.dwarf5.o");
    vec![
        (build_debug_line(), None),
        (elf32_section(&d4, ".debug_line").to_vec(), None),
        (elf32_section(&d5, ".debug_line").to_vec(), Some(elf32_section(&d5, ".debug_line_str").to_vec())),
    ]
}

fn check_debug_line(section: &[u8], line_str: Option<&[u8]>, address_size: u8) {
    let mut opts = DebugLineOptions::new(true, address_size);
    opts.debug_line_str = line_str;
    opts.debug_str = line_str;
    let result = parse_debug_line(section, &opts);
    assert!(result.rows.iter().all(|r| (r.file as usize) < result.files.len()));
    assert!(result.rows.len() <= section.len());
}

#[test]
fn truncated_debug_line_sections_report_errors() {
    for (section, line_str) in debug_line_inputs() {
        // Unit boundaries: a cut exactly there is a valid (shorter) section.
        let mut boundaries = vec![0usize];
        let mut pos = 0usize;
        while pos + 4 <= section.len() {
            let len = u32::from_le_bytes(section[pos..pos + 4].try_into().unwrap());
            pos += if len == 0xffff_ffff {
                12 + u64::from_le_bytes(section[pos + 4..pos + 12].try_into().unwrap()) as usize
            } else {
                4 + len as usize
            };
            boundaries.push(pos);
        }
        let opts = DebugLineOptions { debug_line_str: line_str.as_deref(), ..DebugLineOptions::new(true, 4) };
        for len in 0..section.len() {
            let result = parse_debug_line(&section[..len], &opts);
            if !boundaries.contains(&len) {
                assert!(!result.errors.is_empty(), "no error for a {len}-byte prefix");
            }
            check_debug_line(&section[..len], line_str.as_deref(), 4);
        }
    }
}

#[test]
fn corrupted_debug_line_sections_never_panic() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for (section, line_str) in debug_line_inputs() {
        let mut copy = section.clone();
        for i in 0..section.len() {
            for value in [0x00, 0xff, 0x80, 0x7f, 0x01, section[i] ^ 0x40] {
                copy[i] = value;
                for address_size in [0, 1, 4, 8, 200] {
                    check_debug_line(&copy, line_str.as_deref(), address_size);
                }
            }
            copy[i] = section[i];
        }
        for _ in 0..5000 {
            let mut copy = section.clone();
            for _ in 0..1 + rng.below(6) {
                let at = rng.below(copy.len());
                copy[at] = rng.next() as u8;
            }
            check_debug_line(&copy, line_str.as_deref(), 4);
            check_debug_line(&copy, None, 8);
        }
    }
}

#[test]
fn pathological_debug_line_programs_terminate() {
    // DWARF 5 file table with a zero-width entry format and an absurd entry count.
    let mut w = Writer::new(true);
    w.u32(0).u16(5).u8(&[4, 0]).u32(0);
    let h = w.len();
    w.u8(&[1, 1, 1, -5, 14, 1]); // opcode_base 1: no standard opcode lengths
    w.u8(&[0]).uleb(0); // no directory format, 0 directories
    w.u8(&[1]).uleb(1).uleb(0x19); // file format: (path, flag_present)
    w.uleb(u64::MAX); // file count
    let v = (w.len() - h) as u64;
    w.patch(8, 4, v);
    let v = (w.len() - 4) as u64;
    w.patch(0, 4, v);
    let result = parse_debug_line(&w.bytes(), &DebugLineOptions::new(true, 4));
    assert_eq!(result.errors.len(), 1, "{:?}", result.errors);

    // Huge advances, file numbers and line deltas wrap instead of overflowing.
    let mut w = Writer::new(true);
    w.u32(0).u16(4).u32(0);
    let h = w.len();
    w.u8(&[255, 7, 1, -128, 255, 13]).u8(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]).u8(&[0, 0]);
    let v = (w.len() - h) as u64;
    w.patch(6, 4, v);
    ext(&mut w, 2, true, |p| {
        p.u64(u64::MAX - 3);
    });
    w.u8(&[2]).uleb(u64::MAX).u8(&[3]).sleb(i64::MAX).u8(&[3]).sleb(i64::MAX).u8(&[4]).uleb(u64::MAX).u8(&[1]);
    w.u8(&[8, 255, 9, 0xff, 0xff, 1]);
    ext(&mut w, 1, true, |_| {});
    let v = (w.len() - 4) as u64;
    w.patch(0, 4, v);
    let result = parse_debug_line(&w.bytes(), &DebugLineOptions::new(true, 8));
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.files, ["<unknown>"]);
    assert!(!result.rows.is_empty());
}

#[test]
fn malformed_intel_hex_never_panics() {
    let valid = to_intel_hex(&(0..300u32).map(|i| (i * 13) as u8).collect::<Vec<_>>(), 0, 300);
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let alphabet: Vec<char> = "0123456789ABCDEFabcdefG:\r\n \t\u{feff}\u{e9}\u{1F600}".chars().collect();

    for len in 0..valid.len() {
        let p = parse_intel_hex(&valid[..len], 64, "t.hex");
        assert!(p.flash_used <= 64);
        assert!(!p.diagnostics.is_empty()); // missing EOF at least
    }
    for _ in 0..5000 {
        let mut chars: Vec<char> = valid.chars().collect();
        for _ in 0..1 + rng.below(5) {
            let at = rng.below(chars.len());
            chars[at] = alphabet[rng.below(alphabet.len())];
        }
        let text: String = chars.into_iter().collect();
        let p = parse_intel_hex(&text, rng.below(400), "t.hex");
        assert!(p.flash_used as usize <= p.flash.len());
    }
    for _ in 0..2000 {
        let bytes: Vec<u8> = (0..rng.below(200)).map(|_| rng.next() as u8).collect();
        let _ = load_program_file(&bytes, "x.hex", 32);
        let _ = load_program_file(&bytes, "x.elf", 32);
    }
    // Addresses far beyond flash, with extended records near the top of the 32-bit space.
    let p = parse_intel_hex(":02000004FFFFFC\n:10FFF000000102030405060708090A0B0C0D0E0F89\n:00000001FF\n", 16, "");
    assert_eq!(p.diagnostics.len(), 1);
    assert_eq!(p.diagnostics[0].message, "16 byte(s) lie beyond the end of flash (16 bytes) and were ignored");
}
