//! Intel HEX reader / writer (I8HEX, I16HEX and I32HEX variants).
//!
//! The reader never panics: malformed records are reported as diagnostics (with their 1-based line
//! number) and skipped, so a partially damaged file still loads as much as possible.

use mcs_core::program::{Diagnostic, LoadedProgram, ProgramFormat, Severity};

use crate::sat_u32;

const REC_DATA: u8 = 0x00;
const REC_EOF: u8 = 0x01;
const REC_EXT_SEGMENT_ADDR: u8 = 0x02;
const REC_START_SEGMENT_ADDR: u8 = 0x03;
const REC_EXT_LINEAR_ADDR: u8 = 0x04;
const REC_START_LINEAR_ADDR: u8 = 0x05;

/// Longest possible record: count + address(2) + type + 255 data bytes + checksum.
const MAX_RECORD_BYTES: usize = 1 + 2 + 1 + 255 + 1;
const BYTES_PER_RECORD: usize = 16;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// ASCII byte -> nibble value (0xFF for non-hex characters).
const HEX_VALUE: [u8; 256] = {
    let mut t = [0xffu8; 256];
    let mut i = 0;
    while i < 10 {
        t[b'0' as usize + i] = i as u8;
        i += 1;
    }
    let mut i = 0;
    while i < 6 {
        t[b'A' as usize + i] = 10 + i as u8;
        t[b'a' as usize + i] = 10 + i as u8;
        i += 1;
    }
    t
};

/// Nibble -> ASCII upper-case hex digit.
const HEX_DIGIT: &[u8; 16] = b"0123456789ABCDEF";

#[inline]
fn report(diagnostics: &mut Vec<Diagnostic>, file: &str, severity: Severity, message: String, line: u32) {
    diagnostics.push(Diagnostic::new(severity, message, file, line, 0));
}

/// Number of UTF-16 code units in valid UTF-8 (the TS reader counts code units).
fn utf16_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xbf => 0, // continuation byte
            0xf0..=0xff => 2, // 4-byte sequence -> surrogate pair
            _ => 1,
        })
        .sum()
}

/// Parse Intel HEX text into a flash image of `flash_size` bytes (unprogrammed bytes are 0xFF).
/// Errors and warnings are reported in `diagnostics`; the function itself never panics.
pub fn parse_intel_hex(text: &str, flash_size: usize, file_name: &str) -> LoadedProgram {
    parse_intel_hex_at(text, flash_size, file_name, 0)
}

/// Like [`parse_intel_hex`] for a device whose flash starts at `flash_base` (STM32: 0x0800_0000):
/// record addresses at or above it are placed relative to it, lower ones are taken as they are.
pub fn parse_intel_hex_at(text: &str, flash_size: usize, file_name: &str, flash_base: u32) -> LoadedProgram {
    let mut program = LoadedProgram::empty(ProgramFormat::Hex, flash_size);
    program.flash_base = flash_base;
    let mut diagnostics = Vec::new();
    let flash = &mut program.flash[..];
    let flash_len = flash.len() as u64;

    let bytes = text.as_bytes();
    let length = bytes.len();
    let mut rec = [0u8; MAX_RECORD_BYTES];
    let mut base: u64 = 0; // from extended segment / linear address records
    let mut used: u64 = 0;
    let mut saw_eof = false;
    let mut overflow_bytes: u64 = 0;
    let mut overflow_line: u32 = 0;
    let mut pos = 0usize;
    let mut line_no: u32 = 0;

    while pos < length && !saw_eof {
        // Locate the line [start, end) without allocating; accept LF, CRLF and CR endings.
        let mut end = bytes[pos..].iter().position(|&c| c == b'\n' || c == b'\r').map_or(length, |i| pos + i);
        let mut start = pos;
        pos = end + if bytes.get(end) == Some(&b'\r') && bytes.get(end + 1) == Some(&b'\n') { 2 } else { 1 };
        line_no = line_no.saturating_add(1);

        loop {
            if start < end && bytes[start] <= 0x20 {
                start += 1;
            } else if bytes[start..end].starts_with(BOM) {
                start += BOM.len();
            } else {
                break;
            }
        }
        while end > start && bytes[end - 1] <= 0x20 {
            end -= 1;
        }
        if start == end {
            continue;
        }

        if bytes[start] != b':' {
            report(&mut diagnostics, file_name, Severity::Error, "Record does not start with \":\"".into(), line_no);
            continue;
        }
        let body = &bytes[start + 1..end];
        let ascii = body.is_ascii();
        let digits = if ascii { body.len() } else { utf16_len(body) };
        if !(10..=MAX_RECORD_BYTES * 2).contains(&digits) || digits & 1 != 0 {
            let msg = format!("Malformed record ({digits} hex digits)");
            report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
            continue;
        }

        // Decode the hex digits and accumulate the checksum in one branch-free pass.
        let n = digits >> 1;
        let mut sum: u32 = 0;
        let mut invalid: u8 = if ascii { 0 } else { 0xff };
        for (dst, &[hi, lo]) in rec[..n].iter_mut().zip(body.as_chunks::<2>().0) {
            let h = HEX_VALUE[usize::from(hi)];
            let l = HEX_VALUE[usize::from(lo)];
            invalid |= h | l;
            let b = (h << 4) | (l & 0x0f);
            *dst = b;
            sum += u32::from(b);
        }
        if invalid & 0xf0 != 0 {
            let msg = "Record contains a non-hexadecimal character".into();
            report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
            continue;
        }

        let count = rec[0];
        if usize::from(count) + 5 != n {
            let msg = format!("Record length mismatch: byte count says {count}, record holds {}", n - 5);
            report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
            continue;
        }
        let found = rec[n - 1];
        if sum & 0xff != 0 {
            let expected = found.wrapping_sub(sum as u8);
            let msg = format!("Checksum mismatch (expected 0x{expected:02X}, found 0x{found:02X})");
            report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
            continue;
        }

        let offset = u64::from(u16::from_be_bytes([rec[1], rec[2]]));
        let rec_type = rec[3];
        match rec_type {
            REC_DATA => {
                let mut addr = base + offset;
                if flash_base != 0 && addr >= u64::from(flash_base) {
                    addr -= u64::from(flash_base);
                }
                let rec_end = addr + u64::from(count);
                if addr < flash_len {
                    let stop = rec_end.min(flash_len);
                    let (a, s) = (addr as usize, stop as usize);
                    flash[a..s].copy_from_slice(&rec[4..4 + (s - a)]);
                    used = used.max(stop);
                }
                if rec_end > flash_len {
                    if overflow_bytes == 0 {
                        overflow_line = line_no;
                    }
                    overflow_bytes += rec_end - addr.max(flash_len);
                }
            }
            REC_EOF => saw_eof = true,
            REC_EXT_SEGMENT_ADDR | REC_EXT_LINEAR_ADDR => {
                if count != 2 {
                    let msg = format!("Address record must hold 2 data bytes, found {count}");
                    report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
                } else {
                    let value = u64::from(u16::from_be_bytes([rec[4], rec[5]]));
                    base = value << if rec_type == REC_EXT_LINEAR_ADDR { 16 } else { 4 };
                }
            }
            REC_START_SEGMENT_ADDR | REC_START_LINEAR_ADDR => {
                if count != 4 {
                    let msg = format!("Start address record must hold 4 data bytes, found {count}");
                    report(&mut diagnostics, file_name, Severity::Error, msg, line_no);
                } else if rec_type == REC_START_LINEAR_ADDR {
                    program.entry = u32::from_be_bytes([rec[4], rec[5], rec[6], rec[7]]);
                } else {
                    let cs = u32::from(u16::from_be_bytes([rec[4], rec[5]]));
                    let ip = u32::from(u16::from_be_bytes([rec[6], rec[7]]));
                    program.entry = cs * 0x10 + ip;
                }
            }
            _ => {
                let msg = format!("Unsupported record type 0x{rec_type:02X} ignored");
                report(&mut diagnostics, file_name, Severity::Warning, msg, line_no);
            }
        }
    }

    if overflow_bytes > 0 {
        let msg = format!(
            "{overflow_bytes} byte(s) lie beyond the end of flash ({flash_size} bytes) and were ignored"
        );
        report(&mut diagnostics, file_name, Severity::Error, msg, overflow_line);
    }
    if !saw_eof {
        report(&mut diagnostics, file_name, Severity::Warning, "Missing end-of-file record".into(), line_no);
    }

    program.flash_used = sat_u32(used);
    program.diagnostics = diagnostics;
    program
}

#[inline]
fn put_byte(out: &mut Vec<u8>, b: u8) {
    out.push(HEX_DIGIT[usize::from(b >> 4)]);
    out.push(HEX_DIGIT[usize::from(b & 0x0f)]);
}

fn put_record(out: &mut Vec<u8>, rec_type: u8, offset: u16, data: &[u8]) {
    let [hi, lo] = offset.to_be_bytes();
    let count = data.len() as u8; // callers pass at most 16 bytes
    out.push(b':');
    put_byte(out, count);
    put_byte(out, hi);
    put_byte(out, lo);
    put_byte(out, rec_type);
    let mut sum = count.wrapping_add(hi).wrapping_add(lo).wrapping_add(rec_type);
    for &b in data {
        sum = sum.wrapping_add(b);
        put_byte(out, b);
    }
    put_byte(out, sum.wrapping_neg());
    out.push(b'\n');
}

/// Serialize `data[start, start + len)` (clamped to `data`) as Intel HEX. Record addresses equal
/// indices into `data`. Emits 16-byte data records, extended linear address records whenever the
/// upper 16 address bits change (records never straddle a 64 KiB boundary) and a final EOF
/// record. Lines end with `"\n"`.
pub fn to_intel_hex(data: &[u8], start: usize, len: usize) -> String {
    let first = start.min(data.len());
    let end = data.len().min(first.saturating_add(len));

    // Exact upper bound of the output size so the text is produced without reallocation.
    let segments = if end > first { ((end - 1) >> 16) - (first >> 16) + 1 } else { 0 };
    let data_records = (end - first).div_ceil(BYTES_PER_RECORD) + segments;
    let mut out = Vec::with_capacity(data_records * (12 + 2 * BYTES_PER_RECORD) + segments * 16 + 12);

    let mut upper = 0usize;
    let mut addr = first;
    while addr < end {
        let hi = addr >> 16;
        if hi != upper {
            upper = hi;
            put_record(&mut out, REC_EXT_LINEAR_ADDR, 0, &[(hi >> 8) as u8, hi as u8]);
        }
        let low = addr & 0xffff;
        let count = BYTES_PER_RECORD.min(end - addr).min(0x10000 - low);
        put_record(&mut out, REC_DATA, low as u16, &data[addr..addr + count]);
        addr += count;
    }
    put_record(&mut out, REC_EOF, 0, &[]);

    String::from_utf8(out).expect("Intel HEX output is ASCII")
}
