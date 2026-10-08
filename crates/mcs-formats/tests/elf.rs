//! ELF loader and DWARF line-table tests (port of `tests/formats/elf.test.ts`).

mod common;

use common::*;
use mcs_core::program::{LineEntry, ProgramFormat, Severity, SymbolKind, SymbolSpace};
use mcs_formats::dwarf::{parse_debug_line, DebugLineOptions, DebugLineRow};
use mcs_formats::{load_program_file, parse_elf};

fn row(address: u64, file: u32, line: i64, is_stmt: bool, end_sequence: bool) -> DebugLineRow {
    DebugLineRow { address, file, line, is_stmt, end_sequence }
}

fn line(address: u32, file: u32, line: u32, is_stmt: bool) -> LineEntry {
    LineEntry { address, file, line, is_stmt }
}

const SYNTH_FILES: [&str; 5] = ["src/main.c", "util.h", "src/gen.c", "C:/proj/src/lib.c", "/abs/x.c"];

// ---------------------------------------------------------------------------------------------
// parse_debug_line
// ---------------------------------------------------------------------------------------------

#[test]
fn debug_line_interprets_dwarf2_and_64bit_dwarf4_programs() {
    let result = parse_debug_line(&build_debug_line(), &DebugLineOptions::new(true, 4));
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.files, SYNTH_FILES);
    assert_eq!(
        result.rows,
        vec![
            row(0x10, 0, 10, true, false),
            row(0x12, 0, 11, true, false),
            row(0x16, 0, 11, false, false),
            row(0x18, 1, 3, true, false),
            row(0x3c, 2, 20, true, false),
            row(0x3c, 2, 15, true, false),
            row(0x3e, 2, 15, true, true),
            row(0x40, 3, 5, false, false),
            row(0x42, 0, 6, false, false),
            row(0x42, 0, 6, false, false),
            row(0x42, 4, 7, false, false),
            row(0x44, 4, 7, false, true),
        ]
    );
}

#[test]
fn debug_line_keeps_earlier_units_and_reports_a_truncated_unit() {
    let good = build_debug_line();
    let unit1_length = u32::from_le_bytes(good[0..4].try_into().unwrap()) as usize + 4;
    let truncated = &good[..unit1_length + 30];
    let result = parse_debug_line(truncated, &DebugLineOptions::new(true, 4));
    assert_eq!(result.rows.len(), 7);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].starts_with(&format!("line table at 0x{unit1_length:x}: ")), "{:?}", result.errors);
}

#[test]
fn debug_line_reports_bad_headers_per_unit() {
    // Version 7 unit followed by a valid unit: the first is reported, the second still decodes.
    let good = build_debug_line();
    let unit1_length = u32::from_le_bytes(good[0..4].try_into().unwrap()) as usize + 4;
    let mut bad = good[..unit1_length].to_vec();
    bad[4] = 7;
    bad.extend_from_slice(&good[unit1_length..]);
    let result = parse_debug_line(&bad, &DebugLineOptions::new(true, 4));
    assert_eq!(result.errors, vec!["line table at 0x0: unsupported line table version 7".to_owned()]);
    assert_eq!(result.rows.len(), 5);

    // Reserved unit length stops decoding.
    let result = parse_debug_line(&[0xf0, 0xff, 0xff, 0xff, 0, 0], &DebugLineOptions::new(true, 4));
    assert_eq!(result.errors, vec!["line table at 0x0: reserved unit length 0xfffffff0".to_owned()]);
}

/// Hand-built DWARF 5 unit: line_strp / strp / string / udata / data16 forms and directory joins.
#[test]
fn debug_line_decodes_dwarf5_entry_formats() {
    let line_str = b"/work\0inc\0a.c\0".to_vec();
    let debug_str = b"xx\0b.h\0".to_vec();
    let mut w = Writer::new(true);
    w.u32(0).u16(5).u8(&[4, 0]); // version, address_size, segment_selector_size
    let hl = w.len();
    w.u32(0);
    let h = w.len();
    w.u8(&[1, 1, 1, -5, 14, 13]); // min_inst, max_ops, default_is_stmt, line_base, line_range, opcode_base
    w.u8(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);
    // directories: (DW_LNCT_path, line_strp)
    w.u8(&[1]).uleb(1).uleb(0x1f).uleb(2).u32(0).u32(6);
    // files: (path, strp|string), (directory_index, udata), (MD5, data16)
    w.u8(&[3]).uleb(1).uleb(0x0e).uleb(2).uleb(0x0f).uleb(5).uleb(0x1e).uleb(3);
    w.u32(3).uleb(1).raw(&[0; 16]); // file 0: inc/b.h (strp)
    w.u32(3).uleb(0).raw(&[0; 16]); // file 1: b.h relative to /work
    w.u32(0).uleb(9).raw(&[0; 16]); // file 2: "xx" with an invalid directory
    let v = (w.len() - h) as u64;
    w.patch(hl, 4, v);
    ext(&mut w, 2, true, |p| {
        p.u32(0x200);
    });
    w.u8(&[4]).uleb(0).u8(&[1]); // file 0
    w.u8(&[4]).uleb(1).u8(&[2]).uleb(2).u8(&[1]); // file 1
    w.u8(&[4]).uleb(2).u8(&[2]).uleb(2).u8(&[1]); // file 2
    w.u8(&[4]).uleb(7).u8(&[2]).uleb(2).u8(&[1]); // undefined file number
    w.u8(&[2]).uleb(2);
    ext(&mut w, 1, true, |_| {});
    let v = (w.len() - 4) as u64;
    w.patch(0, 4, v);
    let section = w.bytes();

    let mut opts = DebugLineOptions::new(true, 4);
    opts.debug_line_str = Some(&line_str);
    opts.debug_str = Some(&debug_str);
    let result = parse_debug_line(&section, &opts);
    assert_eq!(result.errors, Vec::<String>::new());
    assert_eq!(result.files, ["/work/inc/b.h", "/work/b.h", "xx", "<unknown>"]);
    let addrs: Vec<(u64, u32)> = result.rows.iter().map(|r| (r.address, r.file)).collect();
    assert_eq!(addrs, [(0x200, 0), (0x202, 1), (0x204, 2), (0x206, 3), (0x208, 3)]);

    // Missing string section -> per-unit error, no panic.
    let result = parse_debug_line(&section, &DebugLineOptions::new(true, 4));
    assert_eq!(result.rows, vec![]);
    assert_eq!(
        result.errors,
        vec!["line table at 0x0: string form refers to missing .debug_line_str section".to_owned()]
    );
}

// ---------------------------------------------------------------------------------------------
// parse_elf (synthetic AVR image)
// ---------------------------------------------------------------------------------------------

#[test]
fn synthetic_avr_elf_loads_without_diagnostics() {
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 1024, "prog.elf");
    assert_eq!(p.diagnostics, vec![]);
    assert_eq!(p.format, ProgramFormat::Elf);
    assert_eq!(p.entry, 0);
    assert_eq!(p.device.as_deref(), Some("attiny10"));
}

#[test]
fn synthetic_avr_elf_builds_flash_from_segment_lmas() {
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 1024, "prog.elf");
    assert_eq!(p.flash.len(), 1024);
    assert_eq!(&p.flash[..0x20], &text_bytes()[..]);
    assert_eq!(&p.flash[0x20..0x25], &[0xde, 0xad, 0xbe, 0xef, 0xff]);
    assert_eq!(p.flash_used, 0x24);
}

#[test]
fn synthetic_avr_elf_routes_eeprom_fuse_lock_and_ignores_signature() {
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 1024, "prog.elf");
    assert_eq!(p.eeprom.as_deref(), Some(&[0xff, 0xff, 1, 2, 3][..]));
    assert_eq!(p.fuses.as_deref(), Some(&[0xfe][..]));
    assert_eq!(p.lock.as_deref(), Some(&[0xfc][..]));
}

#[test]
fn synthetic_avr_elf_maps_symbols_into_address_spaces_and_sorts_them() {
    use SymbolKind::*;
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 1024, "prog.elf");
    assert_eq!(
        p.symbols,
        vec![
            psym("__vectors", 0, 0, Label, SymbolSpace::Code, true),
            psym("__ctors_end", 0x0e, 0, Label, SymbolSpace::Code, true),
            psym("main", 0x10, 0x10, Func, SymbolSpace::Code, true),
            psym("loop", 0x14, 0, Label, SymbolSpace::Code, false),
            psym("weak_handler", 0x18, 2, Func, SymbolSpace::Code, true),
            psym("counter", 0x40, 2, Object, SymbolSpace::Data, true),
            psym("buffer", 0x44, 2, Object, SymbolSpace::Data, false),
            psym("ee_cfg", 2, 3, Object, SymbolSpace::Eeprom, true),
            psym("F_CPU", 8_000_000, 0, Const, SymbolSpace::None, true),
            psym("__fuse", 0x82_0000, 1, Object, SymbolSpace::None, true),
        ]
    );
}

#[test]
fn synthetic_avr_elf_produces_sorted_deduplicated_line_table() {
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 1024, "prog.elf");
    assert_eq!(p.files, SYNTH_FILES);
    assert_eq!(
        p.lines,
        vec![
            line(0x10, 0, 10, true),
            line(0x12, 0, 11, true),
            line(0x16, 0, 11, false),
            line(0x18, 1, 3, true),
            line(0x3c, 2, 20, true),
            line(0x3c, 2, 15, true),
            line(0x40, 3, 5, false),
            line(0x42, 0, 6, false),
            line(0x42, 4, 7, false),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// parse_elf (robustness)
// ---------------------------------------------------------------------------------------------

#[test]
fn bad_debug_info_becomes_a_warning_and_the_image_still_loads() {
    let corrupt = &build_debug_line()[..20]; // unit length now exceeds the section
    let p = parse_elf(&build_avr_elf(corrupt), 1024, "");
    assert_eq!(severities(&p), [Severity::Warning]);
    assert!(p.diagnostics[0].message.to_lowercase().contains("line"), "{}", p.diagnostics[0].message);
    assert_eq!(p.lines, vec![]);
    assert_eq!(p.flash_used, 0x24);
    assert!(!p.symbols.is_empty());
}

#[test]
fn reports_images_larger_than_flash() {
    let p = parse_elf(&build_avr_elf(&build_debug_line()), 0x22, "big.elf");
    assert_eq!(severities(&p), [Severity::Error]);
    assert_eq!(p.diagnostics[0].file, "big.elf");
    assert!(p.diagnostics[0].message.contains("2 byte"), "{}", p.diagnostics[0].message);
    assert_eq!(&p.flash[0x20..], &[0xde, 0xad]);
    assert_eq!(p.flash_used, 0x22);
}

#[test]
fn rejects_non_elf_and_truncated_input_without_panicking() {
    let p = parse_elf(b"hello world, not an ELF", 64, "");
    assert_eq!(severities(&p), [Severity::Error]);
    assert_eq!(p.diagnostics[0].message, "Not an ELF file (bad magic number)");
    let truncated = parse_elf(&build_avr_elf(&build_debug_line())[..40], 64, "");
    assert_eq!(severities(&truncated), [Severity::Error]);
    assert_eq!(truncated.diagnostics[0].message, "Truncated or malformed ELF file");
    assert_eq!(truncated.flash_used, 0);
}

#[test]
fn falls_back_to_allocated_progbits_sections_without_program_headers() {
    let mut spec = ElfSpec::new(vec![
        section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 0x10, &[1, 2]),
        section(".comment", SHT_PROGBITS, 0, 0, &[9, 9, 9]),
        section(".eeprom", SHT_PROGBITS, SHF_ALLOC, 0x81_0000, &[7]),
    ]);
    spec.e_type = 1; // ET_REL
    let p = parse_elf(&build_elf(&spec), 64, "");
    assert_eq!(severities(&p), [Severity::Warning]); // relocatable object
    assert_eq!(&p.flash[0x10..0x13], &[1, 2, 0xff]);
    assert_eq!(p.flash_used, 0x12);
    assert_eq!(p.eeprom.as_deref(), Some(&[7][..]));
}

#[test]
fn reads_big_endian_elf64_files_from_other_machines_with_a_warning() {
    let p = parse_elf(&build_be64_elf(), 0x200, "");
    assert_eq!(severities(&p), [Severity::Warning]);
    assert!(p.diagnostics[0].message.to_lowercase().contains("machine"));
    assert_eq!(p.entry, 0x100);
    assert_eq!(&p.flash[0x100..0x104], &[9, 8, 7, 6]);
    assert_eq!(p.flash_used, 0x104);
    assert_eq!(p.device, None);
    assert_eq!(
        p.symbols,
        vec![
            psym("start", 0x100, 4, SymbolKind::Func, SymbolSpace::Code, true),
            psym("table", 0x102, 2, SymbolKind::Object, SymbolSpace::Code, false),
        ]
    );
    assert_eq!(p.files, ["start.s"]);
    assert_eq!(p.lines, vec![line(0x100, 0, 3, true)]);
}

#[test]
fn warns_about_avr_segments_outside_the_address_map() {
    let mut spec = ElfSpec::new(vec![section(".odd", SHT_PROGBITS, SHF_ALLOC, 0x90_0000, &[1, 2])]);
    spec.segments = vec![TestSegment { section: 1, vaddr: 0x90_0000, paddr: 0x90_0000 }];
    let p = parse_elf(&build_elf(&spec), 64, "odd.elf");
    assert_eq!(severities(&p), [Severity::Warning]);
    assert_eq!(p.diagnostics[0].message, "Segment 0 at 0x900000 is outside the AVR address map and was ignored");
    assert_eq!(p.flash_used, 0);
}

#[test]
fn load_program_file_dispatches_on_magic_and_extension() {
    let elf = build_avr_elf(&build_debug_line());
    assert_eq!(load_program_file(&elf, "prog.hex", 1024).format, ProgramFormat::Elf);
    assert_eq!(load_program_file(&elf, "prog", 1024).format, ProgramFormat::Elf);

    let hex = load_program_file(b":0400000001020304F2\n:00000001FF\n", "prog.hex", 16);
    assert_eq!(hex.format, ProgramFormat::Hex);
    assert_eq!(hex.diagnostics, vec![]);
    assert_eq!(&hex.flash[..5], &[1, 2, 3, 4, 0xff]);

    // A damaged object file gets one ELF error instead of a HEX error per line.
    let bad = load_program_file(b"garbage\nmore garbage\n", "dir.v2/prog.ELF", 16);
    assert_eq!(bad.format, ProgramFormat::Elf);
    assert_eq!(severities(&bad), [Severity::Error]);
    let as_hex = load_program_file(b"garbage\n", "dir.elf/prog", 16);
    assert_eq!(as_hex.format, ProgramFormat::Hex);
}

// ---------------------------------------------------------------------------------------------
// parse_elf (compiler-generated relocatable objects)
// ---------------------------------------------------------------------------------------------

// Built by clang for i386 (see tests/fixtures/line_fixture.c); no program headers, so these also
// exercise the section fallback. Expected values cross-checked with llvm-dwarfdump --debug-line.
const EXPECTED_C: [(u32, u32); 10] =
    [(0x00, 17), (0x19, 18), (0x20, 19), (0x32, 20), (0x43, 19), (0x4e, 21), (0x60, 12), (0x66, 13), (0x70, 25), (0x8d, 26)];
const EXPECTED_H: [(u32, u32); 2] = [(0xb0, 3), (0xb6, 4)];

fn check_fixture_lines(version: u8, c_file: &str, h_file: &str) {
    let p = parse_elf(&fixture(&format!("line_fixture.dwarf{version}.o")), 0x1000, "fixture.o");
    assert!(!p.has_errors(), "{:?}", p.diagnostics);
    assert!(p.diagnostics.iter().all(|d| !d.message.contains("DWARF")), "{:?}", p.diagnostics);
    assert_eq!(p.flash_used, 0xbe);

    let c = p.files.iter().position(|f| f == c_file).expect("C file") as u32;
    let h = p.files.iter().position(|f| f == h_file).expect("header file") as u32;

    assert!(p.lines.windows(2).all(|w| w[0].address <= w[1].address));
    assert!(p.lines.iter().all(|l| l.line > 0 && l.address < 0xbe));
    for (address, l) in EXPECTED_C {
        assert!(p.lines.contains(&line(address, c, l, true)), "missing C row 0x{address:x}:{l}");
    }
    for (address, l) in EXPECTED_H {
        assert!(p.lines.contains(&line(address, h, l, true)), "missing header row 0x{address:x}:{l}");
    }
    assert!(p.lines.contains(&line(0x27, c, 19, false)));
    assert!(!p.lines.iter().any(|l| l.address == 0x2f)); // line 0 row dropped
    assert_eq!(p.lines.len(), 26);
}

fn check_fixture_symbols(version: u8) {
    let p = parse_elf(&fixture(&format!("line_fixture.dwarf{version}.o")), 0x1000, "");
    let sym = |name: &str| p.symbols.iter().find(|s| s.name == name);
    let accumulate = sym("accumulate").unwrap();
    assert_eq!((accumulate.address, accumulate.kind, accumulate.space, accumulate.global), (0, SymbolKind::Func, SymbolSpace::Code, true));
    let square = sym("square").unwrap();
    assert_eq!((square.address, square.kind, square.global), (0x60, SymbolKind::Func, false));
    let main = sym("main").unwrap();
    assert_eq!((main.address, main.kind, main.global), (0x70, SymbolKind::Func, true));
    let twice = sym("twice").unwrap();
    assert_eq!((twice.address, twice.kind, twice.global), (0xb0, SymbolKind::Func, false));
    let counter = sym("counter").unwrap();
    assert_eq!((counter.kind, counter.size, counter.global), (SymbolKind::Object, 4, true));
    assert!(sym("line_fixture.c").is_none()); // STT_FILE
}

#[test]
fn decodes_the_dwarf5_line_table() {
    check_fixture_lines(5, "/fixture/line_fixture.c", "/fixture/line_fixture.h");
}

#[test]
fn decodes_the_dwarf4_line_table() {
    check_fixture_lines(4, "line_fixture.c", "line_fixture.h");
}

#[test]
fn reads_symbols_from_the_dwarf5_object() {
    check_fixture_symbols(5);
}

#[test]
fn reads_symbols_from_the_dwarf4_object() {
    check_fixture_symbols(4);
}

#[test]
fn fixture_debug_line_matches_direct_parse() {
    // The ELF loader only filters/sorts what the DWARF decoder produces.
    for version in [4, 5] {
        let elf = fixture(&format!("line_fixture.dwarf{version}.o"));
        let mut opts = DebugLineOptions::new(true, 4);
        if version == 5 {
            opts.debug_line_str = Some(elf32_section(&elf, ".debug_line_str"));
        }
        let result = parse_debug_line(elf32_section(&elf, ".debug_line"), &opts);
        assert_eq!(result.errors, Vec::<String>::new());
        let p = parse_elf(&elf, 0x1000, "");
        assert_eq!(result.files, p.files);
        let kept = result.rows.iter().filter(|r| !r.end_sequence && r.line > 0).count();
        assert!(kept >= p.lines.len());
    }
}
