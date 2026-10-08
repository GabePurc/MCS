//! Program image loaders for MCS: Intel HEX, ELF and DWARF line tables.
//!
//! Every loader is total: malformed input never panics. Problems are reported as
//! [`Diagnostic`](mcs_core::program::Diagnostic)s on the returned [`LoadedProgram`] (errors for
//! unusable images, warnings for missing or bad optional information such as debug info).
//!
//! * [`ihex`] — Intel HEX reader ([`parse_intel_hex`]) and writer ([`to_intel_hex`]).
//! * [`elf`] — ELF32/ELF64 loader ([`parse_elf`]) with AVR address-space conventions.
//! * [`dwarf`] — DWARF 2–5 `.debug_line` interpreter used by the ELF loader.

use std::borrow::Cow;

use mcs_core::program::LoadedProgram;

pub mod dwarf;
pub mod elf;
pub mod ihex;

pub use elf::{parse_elf, EM_AVR};
pub use ihex::{parse_intel_hex, to_intel_hex};

const ELF_MAGIC: &[u8; 4] = b"\x7fELF";

/// Extensions that select the ELF loader even when the magic number is missing, so a damaged
/// object file gets a single "not an ELF file" error instead of one HEX error per line.
const ELF_EXTENSIONS: [&str; 5] = ["elf", "o", "obj", "out", "axf"];

/// Load a program file, picking the loader from its content and name: data starting with the ELF
/// magic number (or a file named `*.elf`, `*.o`, `*.obj`, `*.out`, `*.axf`) goes to [`parse_elf`],
/// anything else is decoded as (lossy UTF-8) Intel HEX text by [`parse_intel_hex`].
pub fn load_program_file(bytes: &[u8], file_name: &str, flash_size: usize) -> LoadedProgram {
    if bytes.starts_with(ELF_MAGIC) || has_elf_extension(file_name) {
        parse_elf(bytes, flash_size, file_name)
    } else {
        parse_intel_hex(&String::from_utf8_lossy(bytes), flash_size, file_name)
    }
}

fn has_elf_extension(file_name: &str) -> bool {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && ELF_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e))
    })
}

/// Drop one leading UTF-8 byte order mark (what a default `TextDecoder` does).
#[inline]
pub(crate) fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// Decode a string from an object file: lossy UTF-8 (U+FFFD for invalid sequences) without a
/// leading BOM. Borrows when the bytes are valid UTF-8.
#[inline]
pub(crate) fn decode_utf8(bytes: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(strip_bom(bytes))
}

/// Saturating conversion for sizes/counts reported in `u32` fields.
#[inline]
pub(crate) fn sat_u32<T: TryInto<u32>>(v: T) -> u32 {
    v.try_into().unwrap_or(u32::MAX)
}
