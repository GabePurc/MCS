//! Intel HEX reader/writer tests (port of `tests/formats/ihex.test.ts`).

use mcs_core::program::{Diagnostic, LoadedProgram, ProgramFormat, Severity};
use mcs_formats::{parse_intel_hex, to_intel_hex};

fn errors(p: &LoadedProgram) -> Vec<&Diagnostic> {
    p.diagnostics.iter().filter(|d| d.severity == Severity::Error).collect()
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect()
}

fn checksum_of(count: u8, addr: u16, rec_type: u8, data: &[u8]) -> String {
    let mut sum = u32::from(count) + u32::from(addr >> 8) + u32::from(addr & 0xff) + u32::from(rec_type);
    sum += data.iter().map(|&b| u32::from(b)).sum::<u32>();
    format!("{:02X}", (sum as u8).wrapping_neg())
}

// ---------------------------------------------------------------------------------------------
// parse_intel_hex
// ---------------------------------------------------------------------------------------------

#[test]
fn loads_data_records_into_a_0xff_filled_flash_image() {
    let text = [":0400000001020304F2", ":02000600AABB93", ":00000001FF", ""].join("\r\n");
    let p = parse_intel_hex(&text, 16, "a.hex");
    assert_eq!(p.format, ProgramFormat::Hex);
    assert_eq!(p.diagnostics, vec![]);
    let mut expected = vec![1, 2, 3, 4, 0xff, 0xff, 0xaa, 0xbb];
    expected.extend([0xff; 8]);
    assert_eq!(p.flash, expected);
    assert_eq!(p.flash_used, 8);
    assert!(p.symbols.is_empty());
    assert!(p.lines.is_empty());
}

#[test]
fn reports_checksum_errors_with_the_line_number_and_skips_the_record() {
    let text = ":0400000001020304F2\n:02000600AABB94\n:00000001FF\n";
    let p = parse_intel_hex(text, 16, "bad.hex");
    assert_eq!(errors(&p).len(), 1);
    let d = &p.diagnostics[0];
    assert_eq!((d.severity, d.file.as_str(), d.line), (Severity::Error, "bad.hex", 2));
    assert_eq!(d.message, "Checksum mismatch (expected 0x93, found 0x94)");
    assert_eq!(p.flash[6], 0xff);
    assert_eq!(p.flash_used, 4);
}

#[test]
fn reports_malformed_lines_without_panicking() {
    let text = "garbage\n:0400000001020304\n:04000000010203G4F2\n:0300000001020304F2\n:00000001FF";
    let p = parse_intel_hex(text, 16, "");
    assert_eq!(errors(&p).iter().map(|d| d.line).collect::<Vec<_>>(), [1, 2, 3, 4]);
    let messages: Vec<&str> = errors(&p).iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "Record does not start with \":\"",
            "Record length mismatch: byte count says 4, record holds 3",
            "Record contains a non-hexadecimal character",
            "Record length mismatch: byte count says 3, record holds 4",
        ]
    );
    assert_eq!(p.flash_used, 0);
}

#[test]
fn counts_digits_in_utf16_code_units_like_the_reference() {
    // 'é' is one UTF-16 code unit (two UTF-8 bytes): 11 digits -> malformed, not "non-hex".
    let p = parse_intel_hex(":00000001FF\u{e9}\n", 16, "");
    assert_eq!(p.diagnostics[0].message, "Malformed record (11 hex digits)");
    // An astral character is two code units: 12 digits -> length OK, then non-hex.
    let p = parse_intel_hex(":00000001FF\u{1F600}\n", 16, "");
    assert_eq!(p.diagnostics[0].message, "Record contains a non-hexadecimal character");
    // Leading BOM and whitespace are ignored.
    let p = parse_intel_hex("\u{feff}  :00000001FF  \n", 16, "");
    assert_eq!(p.diagnostics, vec![]);
}

#[test]
fn warns_when_eof_is_missing_and_ignores_records_after_eof() {
    let p = parse_intel_hex(":0400000001020304F2\n", 16, "");
    assert_eq!(p.diagnostics.len(), 1);
    assert_eq!(p.diagnostics[0].severity, Severity::Warning);
    assert_eq!(p.diagnostics[0].message, "Missing end-of-file record");
    let p = parse_intel_hex(":00000001FF\n:0400000001020304F2\n", 16, "");
    assert_eq!(p.diagnostics, vec![]);
    assert_eq!(p.flash_used, 0);
}

#[test]
fn applies_extended_linear_and_extended_segment_addresses() {
    let linear = ":020000040001F9\n:02000200CAFE34\n:00000001FF\n";
    let p = parse_intel_hex(linear, 0x20000, "");
    assert_eq!(p.diagnostics, vec![]);
    assert_eq!(p.flash[0x10002], 0xca);
    assert_eq!(p.flash[0x10003], 0xfe);
    assert_eq!(p.flash_used, 0x10004);

    let segment = ":020000021000EC\n:0100050042B8\n:00000001FF\n"; // 0x1000 * 16 + 5
    let s = parse_intel_hex(segment, 0x20000, "");
    assert_eq!(s.diagnostics, vec![]);
    assert_eq!(s.flash[0x10005], 0x42);
}

#[test]
fn sets_the_entry_point_from_start_address_records() {
    assert_eq!(parse_intel_hex(":0400000500000123D3\n:00000001FF\n", 16, "").entry, 0x123);
    assert_eq!(parse_intel_hex(":0400000300100005E4\n:00000001FF\n", 16, "").entry, 0x105);
}

#[test]
fn flags_bytes_beyond_the_flash_size_and_keeps_the_in_range_part() {
    let p = parse_intel_hex(":0400060001020304EC\n:00000001FF\n", 8, "");
    // 0x0006: 01 02 fit, 03 04 do not
    assert_eq!(&p.flash[6..], &[1, 2]);
    assert_eq!(p.flash_used, 8);
    let errs = errors(&p);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].line, 1);
    assert_eq!(errs[0].message, "2 byte(s) lie beyond the end of flash (8 bytes) and were ignored");
}

#[test]
fn reports_bad_address_records_and_unsupported_types() {
    let text = ":0100000400FB\n:0100000300FC\n:00000006FA\n:00000001FF\n";
    let p = parse_intel_hex(text, 16, "");
    let got: Vec<(Severity, u32, &str)> =
        p.diagnostics.iter().map(|d| (d.severity, d.line, d.message.as_str())).collect();
    assert_eq!(
        got,
        [
            (Severity::Error, 1, "Address record must hold 2 data bytes, found 1"),
            (Severity::Error, 2, "Start address record must hold 4 data bytes, found 1"),
            (Severity::Warning, 3, "Unsupported record type 0x06 ignored"),
        ]
    );
}

// ---------------------------------------------------------------------------------------------
// to_intel_hex
// ---------------------------------------------------------------------------------------------

#[test]
fn writes_16_byte_records_and_an_eof_record() {
    let data: Vec<u8> = (0..20).collect();
    let text = to_intel_hex(&data, 0, data.len());
    assert_eq!(
        text.split('\n').collect::<Vec<_>>(),
        [":10000000000102030405060708090A0B0C0D0E0F78", ":0400100010111213A6", ":00000001FF", ""]
    );
}

#[test]
fn honours_start_len_and_emits_extended_linear_address_records_across_64k() {
    let data: Vec<u8> = (0..0x20010usize).map(|i| (i * 7) as u8).collect();
    let text = to_intel_hex(&data, 0xfff8, 0x10010);
    let lines: Vec<&str> = text.trim_end().split('\n').collect();
    assert_eq!(
        lines[0],
        format!(":08FFF800{}{}", hex_bytes(&data[0xfff8..0x10000]), checksum_of(8, 0xfff8, 0, &data[0xfff8..0x10000]))
    );
    assert_eq!(lines[1], ":020000040001F9");
    let ela: Vec<&str> = lines.iter().copied().filter(|l| &l[7..9] == "04").collect();
    assert_eq!(ela, [":020000040001F9", ":020000040002F8"]);
    assert!(lines.iter().all(|l| !l.contains('\r')));

    let back = parse_intel_hex(&text, data.len(), "");
    assert_eq!(back.diagnostics, vec![]);
    assert_eq!(back.flash_used, 0x20008);
    assert_eq!(&back.flash[0xfff8..0x20008], &data[0xfff8..0x20008]);
    assert_eq!(back.flash[0xfff7], 0xff);
    assert_eq!(back.flash[0x20008], 0xff);
}

#[test]
fn round_trips_arbitrary_data() {
    let mut seed: u32 = 12345;
    let data: Vec<u8> = (0..1000)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 24) as u8
        })
        .collect();
    let back = parse_intel_hex(&to_intel_hex(&data, 0, data.len()), 1024, "");
    assert_eq!(back.diagnostics, vec![]);
    assert_eq!(back.flash_used, 1000);
    assert_eq!(&back.flash[..1000], &data[..]);
}

#[test]
fn produces_only_an_eof_record_for_empty_input() {
    assert_eq!(to_intel_hex(&[], 0, 0), ":00000001FF\n");
    // Out-of-range start/len are clamped.
    assert_eq!(to_intel_hex(&[1, 2, 3], 10, 5), ":00000001FF\n");
    assert_eq!(to_intel_hex(&[1, 2, 3], 1, usize::MAX), ":020001000203F8\n:00000001FF\n");
}
