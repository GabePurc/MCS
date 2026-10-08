//! ELF loader (ELF32/ELF64, either byte order) producing a [`LoadedProgram`]: memory images from the
//! loadable segments, symbols from `.symtab`, source line mapping from DWARF `.debug_line` and the
//! target device from avr-libc's `.note.gnu.avr.deviceinfo` note.
//!
//! AVR (`e_machine` 83) physical address conventions used by avr-ld:
//!   0x000000 flash, 0x800000 SRAM (data space), 0x810000 EEPROM, 0x820000 fuses,
//!   0x830000 lock bits, 0x840000 signature.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use mcs_core::program::{
    Diagnostic, LineEntry, LoadedProgram, ProgramFormat, ProgramSymbol, Severity, SymbolKind, SymbolSpace,
};

use crate::dwarf::{parse_debug_line, DebugLineOptions, DebugLineRow};
use crate::{decode_utf8, sat_u32, strip_bom};

pub const EM_AVR: u16 = 83;

const AVR_DATA_BASE: u64 = 0x80_0000;
const AVR_EEPROM_BASE: u64 = 0x81_0000;
const AVR_FUSE_BASE: u64 = 0x82_0000;
const AVR_LOCK_BASE: u64 = 0x83_0000;
const AVR_SIGNATURE_BASE: u64 = 0x84_0000;
const AVR_SIGNATURE_END: u64 = 0x85_0000;

const ET_REL: u16 = 1;
const PT_LOAD: u32 = 1;

const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_NOBITS: u32 = 8;
const SHT_DYNSYM: u32 = 11;
const SHF_ALLOC: u64 = 0x2;
const SHF_COMPRESSED: u64 = 0x800;

const SHN_UNDEF: u16 = 0;
const SHN_LORESERVE: u16 = 0xff00;
const SHN_ABS: u16 = 0xfff1;
const SHN_COMMON: u16 = 0xfff2;
const SHN_XINDEX: u16 = 0xffff;

const STB_LOCAL: u8 = 0;
const STT_NOTYPE: u8 = 0;
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;
const STT_FILE: u8 = 4;
const STT_COMMON: u8 = 5;

// ---------------------------------------------------------------------------------------------
// Raw ELF structure
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct OutOfBounds;

/// Bounds-checked fixed-offset field access in the file's byte order.
#[derive(Clone, Copy)]
struct Fields<'a> {
    bytes: &'a [u8],
    le: bool,
}

impl Fields<'_> {
    #[inline]
    fn array<const N: usize>(self, off: u64) -> Result<[u8; N], OutOfBounds> {
        usize::try_from(off)
            .ok()
            .and_then(|o| self.bytes.get(o..))
            .and_then(|s| s.first_chunk::<N>())
            .copied()
            .ok_or(OutOfBounds)
    }

    fn u8(self, off: u64) -> Result<u8, OutOfBounds> {
        self.array::<1>(off).map(|[b]| b)
    }

    fn u16(self, off: u64) -> Result<u16, OutOfBounds> {
        let a = self.array(off)?;
        Ok(if self.le { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) })
    }

    fn u32(self, off: u64) -> Result<u32, OutOfBounds> {
        let a = self.array(off)?;
        Ok(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }

    fn u64(self, off: u64) -> Result<u64, OutOfBounds> {
        let a = self.array(off)?;
        Ok(if self.le { u64::from_le_bytes(a) } else { u64::from_be_bytes(a) })
    }

    /// Address/offset-sized field: 8 bytes in ELF64, 4 in ELF32.
    fn word(self, is64: bool, off: u64) -> Result<u64, OutOfBounds> {
        if is64 {
            self.u64(off)
        } else {
            self.u32(off).map(u64::from)
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Section<'a> {
    /// Raw name bytes (decoded only for messages).
    name: &'a [u8],
    sh_type: u32,
    flags: u64,
    addr: u64,
    offset: u64,
    size: u64,
    link: u32,
    entsize: u64,
}

impl Section<'_> {
    fn is_named(&self, name: &[u8]) -> bool {
        strip_bom(self.name) == name
    }
}

/// "Section <name>" for messages.
struct SectionLabel<'s, 'a>(&'s Section<'a>);

impl fmt::Display for SectionLabel<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = decode_utf8(self.0.name);
        f.write_str("Section ")?;
        f.write_str(if name.is_empty() { "(unnamed)" } else { &name })
    }
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    p_type: u32,
    offset: u64,
    paddr: u64,
    filesz: u64,
}

struct ElfFile<'a> {
    bytes: &'a [u8],
    is64: bool,
    le: bool,
    e_type: u16,
    machine: u16,
    entry: u64,
    segments: Vec<Segment>,
    sections: Vec<Section<'a>>,
}

impl<'a> ElfFile<'a> {
    fn section_named(&self, name: &[u8]) -> Option<&Section<'a>> {
        self.sections.iter().find(|s| s.is_named(name))
    }

    /// File contents of a section (empty for SHT_NOBITS).
    fn section_data(&self, sec: &Section<'_>) -> Result<&'a [u8], String> {
        if sec.sh_type == SHT_NOBITS {
            return Ok(&[]);
        }
        file_range(sec.offset, sec.size, self.bytes.len())
            .map(|r| &self.bytes[r])
            .ok_or_else(|| format!("{} extends beyond end of file", SectionLabel(sec)))
    }
}

enum ElfError {
    /// A header field lies outside the file.
    Truncated,
    Format(String),
}

impl From<OutOfBounds> for ElfError {
    fn from(_: OutOfBounds) -> Self {
        ElfError::Truncated
    }
}

/// `offset..offset + size` when it lies inside a file of `len` bytes.
fn file_range(offset: u64, size: u64, len: usize) -> Option<Range<usize>> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(usize::try_from(size).ok()?)?;
    (end <= len).then_some(start..end)
}

/// Whether a table of `count` entries of `entsize` bytes at `offset` fits in `len` bytes.
fn table_fits(offset: u64, count: u64, entsize: u16, len: usize) -> bool {
    u128::from(offset) + u128::from(count) * u128::from(entsize) <= len as u128
}

/// NUL-terminated string at `offset`, cut at `end` (empty when `offset >= end`).
fn cstr(bytes: &[u8], offset: u64, end: usize) -> &[u8] {
    let end = end.min(bytes.len());
    match usize::try_from(offset).ok().filter(|&o| o < end) {
        Some(start) => {
            let s = &bytes[start..end];
            s.iter().position(|&b| b == 0).map_or(s, |stop| &s[..stop])
        }
        None => &[],
    }
}

fn read_elf(bytes: &[u8]) -> Result<ElfFile<'_>, ElfError> {
    if bytes.len() < 16 || !bytes.starts_with(b"\x7fELF") {
        return Err(ElfError::Format("Not an ELF file (bad magic number)".into()));
    }
    let class = bytes[4];
    let data = bytes[5];
    if class != 1 && class != 2 {
        return Err(ElfError::Format(format!("Unsupported ELF class {class}")));
    }
    if data != 1 && data != 2 {
        return Err(ElfError::Format(format!("Unsupported ELF data encoding {data}")));
    }
    let is64 = class == 2;
    let le = data == 1;
    let r = Fields { bytes, le };
    let size = bytes.len();
    let pick = |a: u64, b: u64| if is64 { a } else { b };

    // ELF header (fields after e_entry shift by 12 bytes in ELF64).
    let e_type = r.u16(16)?;
    let machine = r.u16(18)?;
    let entry = r.word(is64, 24)?;
    let phoff = r.word(is64, pick(32, 28))?;
    let shoff = r.word(is64, pick(40, 32))?;
    let h = pick(52, 40); // offset of e_ehsize
    let phentsize = r.u16(h + 2)?;
    let mut phnum = u64::from(r.u16(h + 4)?);
    let shentsize = r.u16(h + 6)?;
    let mut shnum = u64::from(r.u16(h + 8)?);
    let mut shstrndx = u64::from(r.u16(h + 10)?);

    // Section headers (with the extended numbering escapes stored in section 0).
    let mut sections = Vec::new();
    if shoff != 0 {
        if u64::from(shentsize) < pick(64, 40) {
            return Err(ElfError::Format(format!("Invalid section header size {shentsize}")));
        }
        if shnum == 0 {
            shnum = r.word(is64, shoff.saturating_add(pick(32, 20)))?; // sh_size of section 0
        }
        if shstrndx == u64::from(SHN_XINDEX) {
            shstrndx = r.u32(shoff.saturating_add(pick(40, 24)))?.into(); // sh_link of section 0
        }
        if phnum == 0xffff {
            phnum = r.u32(shoff.saturating_add(pick(44, 28)))?.into(); // sh_info of section 0
        }
        if !table_fits(shoff, shnum, shentsize, size) {
            return Err(ElfError::Format("Section header table extends beyond end of file".into()));
        }
        // shnum * shentsize <= file size, so these products cannot overflow.
        sections.reserve_exact(shnum as usize);
        let mut name_offsets = Vec::with_capacity(shnum as usize);
        for i in 0..shnum {
            let o = shoff + i * u64::from(shentsize);
            name_offsets.push(r.u32(o)?);
            sections.push(if is64 {
                Section {
                    name: &[],
                    sh_type: r.u32(o + 4)?,
                    flags: r.u64(o + 8)?,
                    addr: r.u64(o + 16)?,
                    offset: r.u64(o + 24)?,
                    size: r.u64(o + 32)?,
                    link: r.u32(o + 40)?,
                    entsize: r.u64(o + 56)?,
                }
            } else {
                Section {
                    name: &[],
                    sh_type: r.u32(o + 4)?,
                    flags: r.u32(o + 8)?.into(),
                    addr: r.u32(o + 12)?.into(),
                    offset: r.u32(o + 16)?.into(),
                    size: r.u32(o + 20)?.into(),
                    link: r.u32(o + 24)?,
                    entsize: r.u32(o + 36)?.into(),
                }
            });
        }
        let shstr = usize::try_from(shstrndx).ok().and_then(|i| sections.get(i)).copied();
        if let Some(shstr) = shstr.filter(|s| s.sh_type != SHT_NOBITS) {
            if let Some(range) = file_range(shstr.offset, shstr.size, size) {
                let strtab = &bytes[range];
                for (sec, &off) in sections.iter_mut().zip(&name_offsets) {
                    sec.name = cstr(strtab, off.into(), strtab.len());
                }
            }
        }
    }

    // Program headers.
    let mut segments = Vec::new();
    if phoff != 0 && phnum > 0 {
        if u64::from(phentsize) < pick(56, 32) {
            return Err(ElfError::Format(format!("Invalid program header size {phentsize}")));
        }
        if !table_fits(phoff, phnum, phentsize, size) {
            return Err(ElfError::Format("Program header table extends beyond end of file".into()));
        }
        segments.reserve_exact(phnum as usize);
        for i in 0..phnum {
            let o = phoff + i * u64::from(phentsize);
            segments.push(if is64 {
                Segment { p_type: r.u32(o)?, offset: r.u64(o + 8)?, paddr: r.u64(o + 24)?, filesz: r.u64(o + 32)? }
            } else {
                Segment {
                    p_type: r.u32(o)?,
                    offset: r.u32(o + 4)?.into(),
                    paddr: r.u32(o + 12)?.into(),
                    filesz: r.u32(o + 16)?.into(),
                }
            });
        }
    }

    Ok(ElfFile { bytes, is64, le, e_type, machine, entry, segments, sections })
}

// ---------------------------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------------------------

#[inline]
fn report(diagnostics: &mut Vec<Diagnostic>, file: &str, severity: Severity, message: String) {
    diagnostics.push(Diagnostic::new(severity, message, file, 0, 0));
}

/// Load an ELF executable or object. Never panics: problems are reported in `diagnostics`
/// (errors for unusable images, warnings for missing/bad optional information).
pub fn parse_elf(bytes: &[u8], flash_size: usize, file_name: &str) -> LoadedProgram {
    let mut program = LoadedProgram::empty(ProgramFormat::Elf, flash_size);

    let elf = match read_elf(bytes) {
        Ok(elf) => elf,
        Err(err) => {
            let message = match err {
                ElfError::Truncated => "Truncated or malformed ELF file".to_owned(),
                ElfError::Format(message) => message,
            };
            report(&mut program.diagnostics, file_name, Severity::Error, message);
            return program;
        }
    };

    let diagnostics = &mut program.diagnostics;
    let is_avr = elf.machine == EM_AVR;
    if !is_avr {
        let msg = format!("ELF machine type {} is not AVR ({EM_AVR}); loading anyway", elf.machine);
        report(diagnostics, file_name, Severity::Warning, msg);
    }
    if elf.e_type == ET_REL {
        let msg = "ELF file is a relocatable object (not linked); section addresses are not final".into();
        report(diagnostics, file_name, Severity::Warning, msg);
    }
    match u32::try_from(elf.entry) {
        Ok(entry) => program.entry = entry,
        Err(_) => {
            let msg = format!("Entry point 0x{:x} does not fit in 32 bits; using 0", elf.entry);
            report(diagnostics, file_name, Severity::Warning, msg);
        }
    }

    load_image(&elf, &mut program, is_avr, file_name);

    match read_symbols(&elf, is_avr) {
        Ok((symbols, skipped)) => {
            program.symbols = symbols;
            if skipped > 0 {
                let msg = format!("{skipped} symbol(s) with addresses above 32 bits were ignored");
                report(&mut program.diagnostics, file_name, Severity::Warning, msg);
            }
        }
        Err(e) => {
            let msg = format!("Failed to read symbol table: {e}");
            report(&mut program.diagnostics, file_name, Severity::Warning, msg);
        }
    }

    if is_avr {
        // Device info is optional and purely informational.
        program.device = read_avr_device(&elf);
    }

    if let Some(debug_line) = elf.section_named(b".debug_line") {
        if debug_line.sh_type != SHT_NOBITS && debug_line.size > 0 {
            load_line_info(&elf, debug_line, &mut program, file_name);
        }
    }

    program
}

fn load_line_info(elf: &ElfFile<'_>, debug_line: &Section<'_>, program: &mut LoadedProgram, file_name: &str) {
    let diagnostics = &mut program.diagnostics;
    if debug_line.flags & SHF_COMPRESSED != 0 {
        let msg = "Compressed debug sections are not supported; source line information unavailable".into();
        report(diagnostics, file_name, Severity::Warning, msg);
        return;
    }
    let sections = (|| -> Result<_, String> {
        let optional = |name: &[u8]| elf.section_named(name).map(|s| elf.section_data(s)).transpose();
        Ok((elf.section_data(debug_line)?, optional(b".debug_line_str")?, optional(b".debug_str")?))
    })();
    let (data, debug_line_str, debug_str) = match sections {
        Ok(s) => s,
        Err(e) => {
            report(diagnostics, file_name, Severity::Warning, format!("Failed to read DWARF line info: {e}"));
            return;
        }
    };

    let opts =
        DebugLineOptions { little_endian: elf.le, address_size: if elf.is64 { 8 } else { 4 }, debug_line_str, debug_str };
    let result = parse_debug_line(data, &opts);
    for e in &result.errors {
        report(diagnostics, file_name, Severity::Warning, format!("Bad DWARF line info: {e}"));
    }
    let (lines, skipped) = build_line_table(&result.rows);
    if skipped > 0 {
        let msg = format!("{skipped} line table row(s) with addresses above 32 bits were ignored");
        report(diagnostics, file_name, Severity::Warning, msg);
    }
    program.files = result.files;
    program.lines = lines;
}

/// Collects pieces of a small, variably sized memory (EEPROM/fuses/lock); gaps read as 0xFF.
#[derive(Default)]
struct SparseImage<'a> {
    chunks: Vec<(usize, &'a [u8])>,
    end: usize,
}

impl<'a> SparseImage<'a> {
    /// `offset` is below 0x10000 (one AVR address window) and `data` lies in the file, so the
    /// image size is bounded by the file size.
    fn add(&mut self, offset: u64, data: &'a [u8]) {
        let offset = offset as usize;
        self.end = self.end.max(offset + data.len());
        self.chunks.push((offset, data));
    }

    fn build(&self) -> Option<Vec<u8>> {
        if self.chunks.is_empty() {
            return None;
        }
        let mut out = vec![0xff; self.end];
        for &(offset, data) in &self.chunks {
            out[offset..offset + data.len()].copy_from_slice(data);
        }
        Some(out)
    }
}

/// Routes loadable bytes by load (physical) address into flash / EEPROM / fuses / lock.
struct ImageLoader<'p, 'a> {
    flash: &'p mut [u8],
    diagnostics: &'p mut Vec<Diagnostic>,
    file: &'p str,
    is_avr: bool,
    used: usize,
    eeprom: SparseImage<'a>,
    fuses: SparseImage<'a>,
    lock: SparseImage<'a>,
}

impl<'a> ImageLoader<'_, 'a> {
    fn report(&mut self, severity: Severity, message: String) {
        report(self.diagnostics, self.file, severity, message);
    }

    fn write_flash(&mut self, addr: u64, data: &[u8], what: &dyn fmt::Display) {
        let flash_size = self.flash.len() as u128;
        let start = u128::from(addr);
        let end = start + data.len() as u128;
        if end > flash_size {
            let lost = end - start.max(flash_size);
            let msg = format!("{what}: {lost} byte(s) beyond the end of flash ({flash_size} bytes) were ignored");
            self.report(Severity::Error, msg);
        }
        let stop = end.min(flash_size);
        if stop > start {
            // start < stop <= flash length, so both fit in usize.
            let (a, s) = (start as usize, stop as usize);
            self.flash[a..s].copy_from_slice(&data[..s - a]);
            self.used = self.used.max(s);
        }
    }

    fn place(&mut self, lma: u64, data: &'a [u8], what: &dyn fmt::Display) {
        if data.is_empty() {
            return;
        }
        if !self.is_avr || lma < AVR_DATA_BASE {
            self.write_flash(lma, data, what);
        } else if lma < AVR_EEPROM_BASE {
            // SRAM run-time image; its initial values load from flash.
        } else if lma < AVR_FUSE_BASE {
            self.eeprom.add(lma - AVR_EEPROM_BASE, data);
        } else if lma < AVR_LOCK_BASE {
            self.fuses.add(lma - AVR_FUSE_BASE, data);
        } else if lma < AVR_SIGNATURE_BASE {
            self.lock.add(lma - AVR_LOCK_BASE, data);
        } else if lma < AVR_SIGNATURE_END {
            // Device signature: informational only.
        } else {
            let msg = format!("{what} at 0x{lma:x} is outside the AVR address map and was ignored");
            self.report(Severity::Warning, msg);
        }
    }
}

struct SegmentLabel(usize);

impl fmt::Display for SegmentLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Segment {}", self.0)
    }
}

fn load_image(elf: &ElfFile<'_>, program: &mut LoadedProgram, is_avr: bool, file: &str) {
    let mut loader = ImageLoader {
        flash: &mut program.flash,
        diagnostics: &mut program.diagnostics,
        file,
        is_avr,
        used: 0,
        eeprom: SparseImage::default(),
        fuses: SparseImage::default(),
        lock: SparseImage::default(),
    };
    let len = elf.bytes.len();

    let mut loads = elf.segments.iter().filter(|s| s.p_type == PT_LOAD).peekable();
    if loads.peek().is_some() {
        for (i, seg) in loads.enumerate() {
            if seg.filesz == 0 {
                continue; // .bss-like: p_memsz beyond p_filesz is zero-filled at run time
            }
            match file_range(seg.offset, seg.filesz, len) {
                Some(range) => loader.place(seg.paddr, &elf.bytes[range], &SegmentLabel(i)),
                None => loader.report(Severity::Error, format!("Segment {i} extends beyond end of file")),
            }
        }
    } else {
        // No program headers (e.g. relocatable object): use allocated PROGBITS sections at their address.
        for sec in &elf.sections {
            if sec.sh_type != SHT_PROGBITS || sec.flags & SHF_ALLOC == 0 || sec.size == 0 {
                continue;
            }
            let what = SectionLabel(sec);
            match file_range(sec.offset, sec.size, len) {
                Some(range) => loader.place(sec.addr, &elf.bytes[range], &what),
                None => loader.report(Severity::Error, format!("{what} extends beyond end of file")),
            }
        }
    }

    let ImageLoader { used, eeprom, fuses, lock, .. } = loader;
    program.flash_used = sat_u32(used);
    program.eeprom = eeprom.build();
    program.fuses = fuses.build();
    program.lock = lock.build();
}

fn space_order(space: SymbolSpace) -> u8 {
    match space {
        SymbolSpace::Code => 0,
        SymbolSpace::Data => 1,
        SymbolSpace::Eeprom => 2,
        SymbolSpace::None => 3,
    }
}

/// Symbols from `.symtab` (or `.dynsym`), sorted by space then address. Also returns how many
/// symbols were dropped because their address does not fit in 32 bits.
fn read_symbols(elf: &ElfFile<'_>, is_avr: bool) -> Result<(Vec<ProgramSymbol>, usize), String> {
    let symtab = elf
        .sections
        .iter()
        .find(|s| s.sh_type == SHT_SYMTAB)
        .or_else(|| elf.sections.iter().find(|s| s.sh_type == SHT_DYNSYM));
    let Some(symtab) = symtab else { return Ok((Vec::new(), 0)) };
    let strtab = match usize::try_from(symtab.link).ok().and_then(|i| elf.sections.get(i)) {
        Some(sec) => elf.section_data(sec)?,
        None => &[],
    };
    let data = elf.section_data(symtab)?;
    let r = Fields { bytes: data, le: elf.le };
    let entsize = if symtab.entsize != 0 { symtab.entsize } else if elf.is64 { 24 } else { 16 };
    let count = data.len() as u64 / entsize;
    let oob = |_: OutOfBounds| "symbol table entry extends beyond the end of the section".to_owned();
    let mut symbols = Vec::with_capacity((data.len() / 16).min(count as usize));
    let mut skipped = 0usize;

    for i in 1..count {
        let o = i * entsize; // < data.len()
        let (info, shndx, value, size) = if elf.is64 {
            (r.u8(o + 4), r.u16(o + 6), r.u64(o + 8), r.u64(o + 16))
        } else {
            let value = r.u32(o + 4).map(u64::from);
            let size = r.u32(o + 8).map(u64::from);
            (r.u8(o + 12), r.u16(o + 14), value, size)
        };
        let (info, shndx, value, size) = (info.map_err(oob)?, shndx.map_err(oob)?, value.map_err(oob)?, size.map_err(oob)?);
        let st_type = info & 0xf;
        if st_type == STT_SECTION || st_type == STT_FILE {
            continue;
        }
        if shndx == SHN_UNDEF || shndx == SHN_COMMON {
            continue; // no address in this file
        }
        let name = decode_utf8(cstr(strtab, r.u32(o).map_err(oob)?.into(), strtab.len()));
        if name.is_empty() {
            continue;
        }
        let is_abs = shndx == SHN_ABS;
        // Linker/assembler-defined absolute constants such as __SP_H__ or __DATA_REGION_LENGTH__.
        if is_abs && st_type == STT_NOTYPE && name.starts_with("__") {
            continue;
        }

        let mut kind = match st_type {
            STT_FUNC => SymbolKind::Func,
            STT_OBJECT | STT_COMMON => SymbolKind::Object,
            STT_NOTYPE => SymbolKind::Label,
            _ => SymbolKind::Other,
        };
        let mut space = SymbolSpace::Code;
        let mut address = value;
        if is_abs {
            space = SymbolSpace::None;
            kind = SymbolKind::Const;
        } else if is_avr && value >= AVR_DATA_BASE {
            if value < AVR_EEPROM_BASE {
                space = SymbolSpace::Data;
                address = value - AVR_DATA_BASE;
            } else if value < AVR_FUSE_BASE {
                space = SymbolSpace::Eeprom;
                address = value - AVR_EEPROM_BASE;
            } else {
                space = SymbolSpace::None; // fuse / lock / signature objects keep their raw address
            }
        } else if shndx >= SHN_LORESERVE && shndx != SHN_XINDEX {
            space = SymbolSpace::None; // other processor/OS-specific special sections
        }

        let Ok(address) = u32::try_from(address) else {
            skipped += 1;
            continue;
        };
        symbols.push(ProgramSymbol {
            name: name.into_owned(),
            address,
            size: sat_u32(size),
            kind,
            space,
            global: info >> 4 != STB_LOCAL,
        });
    }

    symbols.sort_by_key(|s| (space_order(s.space), s.address)); // stable
    Ok((symbols, skipped))
}

/// Device name from the `.note.gnu.avr.deviceinfo` note emitted by avr-libc's startup code:
/// note name "AVR"; desc = flash start/size, SRAM start/size, EEPROM start/size (6 words), then a
/// string offset table (length word followed by offsets; entry 0 = device name) and a string table.
fn read_avr_device(elf: &ElfFile<'_>) -> Option<String> {
    let sec = elf.section_named(b".note.gnu.avr.deviceinfo")?;
    let data = elf.section_data(sec).ok()?;
    let r = Fields { bytes: data, le: elf.le };
    let len = data.len() as u64;
    let align4 = |n: u64| (n + 3) & !3;

    let mut pos = 0u64;
    while pos + 12 <= len {
        let namesz = u64::from(r.u32(pos).ok()?);
        let descsz = u64::from(r.u32(pos + 4).ok()?);
        let name_start = pos + 12;
        let desc_start = name_start + align4(namesz);
        if desc_start + descsz > len {
            return None;
        }
        if strip_bom(cstr(data, name_start, (name_start + namesz) as usize)) == b"AVR" {
            return device_name_from_desc(&data[desc_start as usize..(desc_start + descsz) as usize], elf.le);
        }
        pos = desc_start + align4(descsz);
    }
    None
}

fn is_valid_device_name(name: &str) -> bool {
    // /^[A-Za-z][A-Za-z0-9_]{1,39}$/
    let b = name.as_bytes();
    (2..=40).contains(&b.len())
        && b[0].is_ascii_alphabetic()
        && b[1..].iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_')
}

fn device_name_from_desc(desc: &[u8], le: bool) -> Option<String> {
    const TABLE: usize = 24; // after the six memory words
    if desc.len() < TABLE + 8 {
        return None;
    }
    let r = Fields { bytes: desc, le };
    let table_length = r.u32(TABLE as u64).ok()?;
    let name_offset = r.u32(TABLE as u64 + 4).ok()?;

    // Implementations disagree on whether the length word counts entries, bytes, or bytes including
    // itself; with N entries the string table always starts at TABLE + 4 + 4N, so try each reading.
    let quarter = i64::from(table_length >> 2);
    let mut counts = [i64::from(table_length), quarter, quarter - 1];
    counts.sort_unstable();
    let mut previous = None;
    for n in counts {
        if previous == Some(n) {
            continue;
        }
        previous = Some(n);
        if n < 1 || (TABLE as i64 + 4 + 4 * n) >= desc.len() as i64 {
            continue;
        }
        let strtab = TABLE + 4 + 4 * n as usize; // < desc.len()
        let Some(at) = usize::try_from(name_offset).ok().and_then(|off| strtab.checked_add(off)) else { continue };
        if at >= desc.len() || (at > strtab && desc[at - 1] != 0) {
            continue; // must start a string
        }
        let name = decode_utf8(cstr(desc, at as u64, desc.len()));
        if is_valid_device_name(&name) {
            return Some(name.to_ascii_lowercase());
        }
    }
    None
}

/// Line rows -> sorted `LineEntry` list without end-of-sequence rows, line-0 rows or exact
/// duplicates (a duplicate's `is_stmt` is merged into the first occurrence). Also returns the
/// number of rows dropped because their address does not fit in 32 bits.
fn build_line_table(rows: &[DebugLineRow]) -> (Vec<LineEntry>, usize) {
    let mut skipped = 0usize;
    let mut entries: Vec<LineEntry> = rows
        .iter()
        .filter(|row| !row.end_sequence && row.line > 0)
        .filter_map(|row| {
            let line = u32::try_from(row.line).ok()?;
            match u32::try_from(row.address) {
                Ok(address) => Some(LineEntry { address, file: row.file, line, is_stmt: row.is_stmt }),
                Err(_) => {
                    skipped += 1;
                    None
                }
            }
        })
        .collect();
    entries.sort_by_key(|e| e.address); // stable: keeps program order within an address

    // Dedupe within each run of equal addresses. Runs are tiny in practice; large ones (only seen in
    // malformed input) switch to a hash map to stay linear.
    const LINEAR_LIMIT: usize = 16;
    let mut seen: HashMap<(u32, u32), usize> = HashMap::new();
    let mut out = 0usize;
    let mut i = 0usize;
    while i < entries.len() {
        let address = entries[i].address;
        let run_start = out;
        let mut run_end = i;
        while run_end < entries.len() && entries[run_end].address == address {
            run_end += 1;
        }
        let large = run_end - i > LINEAR_LIMIT;
        if large {
            seen.clear();
        }
        for k in i..run_end {
            let e = entries[k];
            let existing = if large {
                seen.get(&(e.file, e.line)).copied()
            } else {
                (run_start..out).find(|&j| entries[j].file == e.file && entries[j].line == e.line)
            };
            match existing {
                Some(j) => entries[j].is_stmt |= e.is_stmt,
                None => {
                    if large {
                        seen.insert((e.file, e.line), out);
                    }
                    entries[out] = e;
                    out += 1;
                }
            }
        }
        i = run_end;
    }
    entries.truncate(out);
    (entries, skipped)
}
