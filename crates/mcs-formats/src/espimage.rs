//! ESP-IDF application image reader (`.bin`, as produced by `esptool.py elf2image`) for the ESP32-C3.
//!
//! Layout (ESP-IDF `esp_image_format.h`): an 8-byte header `[magic 0xE9, segment count, SPI mode,
//! SPI speed/size, entry address (u32)]`, a 16-byte extended header (WP pin, SPI pin drives, `chip_id` u16, ...)
//! and then `segment count` segments, each `[load address u32, data length u32, data]`. A checksum byte
//! (padded to 16 bytes) and an optional SHA-256 digest follow; they are ignored. Segments are placed
//! at their load addresses (flash windows, IRAM, DRAM, RTC memory); the part in the instruction flash
//! window is also copied into the flash image for the disassembly view. The image has no symbols or line
//! information.
//!
//! Direct boot starts at the entry address; the 2nd stage bootloader and its MMU setup are not simulated.

use mcs_core::program::{Diagnostic, LoadedProgram, ProgramFormat, ProgramSegment, Severity};

use crate::sat_u32;

pub const ESP_IMAGE_MAGIC: u8 = 0xe9;
const CHIP_ID_ESP32C3: u16 = 5;
const RISCV_DEFAULT_FLASH_BASE: u32 = 0x4200_0000;
const HEADER: usize = 24;
const MAX_SEGMENTS: usize = 16;

/// True if `bytes` start like an ESP application image (magic byte and a plausible segment count).
pub fn looks_like_esp_image(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER && bytes[0] == ESP_IMAGE_MAGIC && (1..=MAX_SEGMENTS as u8).contains(&bytes[1])
}

fn err(p: &mut LoadedProgram, file: &str, msg: impl Into<String>) {
    p.diagnostics.push(Diagnostic::error(msg, file, 0, 0));
}

/// Parses an ESP application image. Never panics; problems are reported in `diagnostics`.
pub fn parse_esp_image(bytes: &[u8], flash_size: usize, file_name: &str, flash_base: Option<u32>) -> LoadedProgram {
    let mut p = LoadedProgram::empty(ProgramFormat::Hex, flash_size);
    let base = flash_base.filter(|&b| b != 0).unwrap_or(RISCV_DEFAULT_FLASH_BASE);
    p.flash_base = base;
    if !looks_like_esp_image(bytes) {
        err(&mut p, file_name, "Not an ESP application image (bad magic number or segment count)");
        return p;
    }
    let nseg = bytes[1] as usize;
    p.entry = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let chip = u16::from_le_bytes([bytes[12], bytes[13]]);
    if chip != CHIP_ID_ESP32C3 {
        p.diagnostics.push(Diagnostic::warning(format!("Image chip id {chip} is not the ESP32-C3 ({CHIP_ID_ESP32C3}); loading anyway"), file_name, 0, 0));
    }
    let mut pos = HEADER;
    let mut used = 0usize;
    for i in 0..nseg {
        let Some(h) = bytes.get(pos..pos + 8) else {
            err(&mut p, file_name, format!("Segment {i}: truncated header"));
            break;
        };
        let addr = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
        pos += 8;
        let Some(data) = bytes.get(pos..pos.saturating_add(len)) else {
            err(&mut p, file_name, format!("Segment {i}: data extends beyond the end of the file"));
            break;
        };
        pos += len;
        if len == 0 {
            continue;
        }
        let end = u64::from(addr) + len as u64;
        if end > u64::from(u32::MAX) + 1 {
            err(&mut p, file_name, format!("Segment {i} does not fit the 32-bit address space"));
            continue;
        }
        if addr >= base && ((addr - base) as usize) < p.flash.len() {
            let off = (addr - base) as usize;
            let n = len.min(p.flash.len() - off);
            p.flash[off..off + n].copy_from_slice(&data[..n]);
            used = used.max(off + n);
        }
        p.segments.push(ProgramSegment { address: addr, data: data.to_vec() });
    }
    p.flash_used = sat_u32(used);
    if p.segments.is_empty() && !p.has_errors() {
        err(&mut p, file_name, "The image contains no segments");
    }
    p.diagnostics.push(Diagnostic::new(Severity::Info, "ESP application image: no symbols or source line information", file_name, 0, 0));
    p
}
