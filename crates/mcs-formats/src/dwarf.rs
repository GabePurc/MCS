//! DWARF `.debug_line` decoder: a complete line-number program interpreter for DWARF versions 2–5
//! (32- and 64-bit DWARF, either byte order). Only the line table is decoded; `.debug_info` is not
//! needed because the line program header carries its own file table.
//!
//! All reads are bounds-checked; a damaged unit is reported in [`DebugLineResult::errors`] and the
//! rows decoded before the damage are kept. The decoder never panics.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::{decode_utf8, strip_bom};

/// One row of the line-number matrix, in program order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugLineRow {
    pub address: u64,
    /// Index into [`DebugLineResult::files`].
    pub file: u32,
    /// Source line; may be 0 ("no line") or even negative in malformed programs.
    pub line: i64,
    pub is_stmt: bool,
    pub end_sequence: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugLineResult {
    /// Files referenced by `rows`, deduplicated across all units, in order of first use.
    pub files: Vec<String>,
    pub rows: Vec<DebugLineRow>,
    /// One message per unit that could not be (fully) decoded; rows decoded before the error are kept.
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct DebugLineOptions<'a> {
    pub little_endian: bool,
    /// Target address size in bytes (used for tombstone detection; DWARF 5 headers override it).
    pub address_size: u8,
    /// `.debug_line_str` contents (`DW_FORM_line_strp`, DWARF 5).
    pub debug_line_str: Option<&'a [u8]>,
    /// `.debug_str` contents (`DW_FORM_strp`).
    pub debug_str: Option<&'a [u8]>,
}

impl DebugLineOptions<'_> {
    /// Options without string sections.
    pub fn new(little_endian: bool, address_size: u8) -> Self {
        Self { little_endian, address_size, debug_line_str: None, debug_str: None }
    }
}

/// Path used for file numbers the unit does not define.
pub const UNKNOWN_FILE: &str = "<unknown>";

// Standard opcodes (DW_LNS_*).
const LNS_COPY: u8 = 1;
const LNS_ADVANCE_PC: u8 = 2;
const LNS_ADVANCE_LINE: u8 = 3;
const LNS_SET_FILE: u8 = 4;
const LNS_SET_COLUMN: u8 = 5;
const LNS_NEGATE_STMT: u8 = 6;
const LNS_SET_BASIC_BLOCK: u8 = 7;
const LNS_CONST_ADD_PC: u8 = 8;
const LNS_FIXED_ADVANCE_PC: u8 = 9;
const LNS_SET_PROLOGUE_END: u8 = 10;
const LNS_SET_EPILOGUE_BEGIN: u8 = 11;
const LNS_SET_ISA: u8 = 12;

// Extended opcodes (DW_LNE_*).
const LNE_END_SEQUENCE: u8 = 1;
const LNE_SET_ADDRESS: u8 = 2;
const LNE_DEFINE_FILE: u8 = 3;

// Line table entry content types (DW_LNCT_*, DWARF 5).
const LNCT_PATH: u64 = 1;
const LNCT_DIRECTORY_INDEX: u64 = 2;

// Attribute forms that may appear in DWARF 5 directory / file entry formats (DW_FORM_*).
const FORM_BLOCK2: u64 = 0x03;
const FORM_BLOCK4: u64 = 0x04;
const FORM_DATA2: u64 = 0x05;
const FORM_DATA4: u64 = 0x06;
const FORM_DATA8: u64 = 0x07;
const FORM_STRING: u64 = 0x08;
const FORM_BLOCK: u64 = 0x09;
const FORM_BLOCK1: u64 = 0x0a;
const FORM_DATA1: u64 = 0x0b;
const FORM_FLAG: u64 = 0x0c;
const FORM_SDATA: u64 = 0x0d;
const FORM_STRP: u64 = 0x0e;
const FORM_UDATA: u64 = 0x0f;
const FORM_SEC_OFFSET: u64 = 0x17;
const FORM_FLAG_PRESENT: u64 = 0x19;
const FORM_STRX: u64 = 0x1a;
const FORM_STRP_SUP: u64 = 0x1d;
const FORM_DATA16: u64 = 0x1e;
const FORM_LINE_STRP: u64 = 0x1f;
const FORM_STRX1: u64 = 0x25;
const FORM_STRX2: u64 = 0x26;
const FORM_STRX3: u64 = 0x27;
const FORM_STRX4: u64 = 0x28;
const FORM_GNU_STR_INDEX: u64 = 0x1f02;
const FORM_GNU_STRP_ALT: u64 = 0x1f21;

/// Global file index not resolved yet.
const UNRESOLVED: u32 = u32::MAX;

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum StrSection {
    LineStr,
    Str,
}

impl StrSection {
    fn name(self) -> &'static str {
        match self {
            StrSection::LineStr => ".debug_line_str",
            StrSection::Str => ".debug_str",
        }
    }
}

/// Decoding error; small and `Copy` so `Result<u8, Error>` stays register-sized.
#[derive(Clone, Copy, Debug)]
enum Error {
    UnexpectedEnd(usize),
    UnterminatedString(usize),
    MissingStrSection(StrSection),
    StrOffsetOutside(u64, StrSection),
    UnsupportedForm(u64),
    ReservedUnitLength(u64),
    UnitBeyondSection,
    UnsupportedVersion(u16),
    HeaderLengthExceedsUnit,
    LineRangeZero,
    OpcodeBaseZero,
    ExtendedOpcodeOverrun(usize),
    TooManyEntries(u64),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::UnexpectedEnd(at) => write!(f, "unexpected end of data at offset 0x{at:x}"),
            Error::UnterminatedString(at) => write!(f, "unterminated string at offset 0x{at:x}"),
            Error::MissingStrSection(s) => write!(f, "string form refers to missing {} section", s.name()),
            Error::StrOffsetOutside(off, s) => write!(f, "string offset 0x{off:x} outside {}", s.name()),
            Error::UnsupportedForm(form) => write!(f, "unsupported attribute form 0x{form:x} in line table header"),
            Error::ReservedUnitLength(len) => write!(f, "reserved unit length 0x{len:x}"),
            Error::UnitBeyondSection => f.write_str("unit extends beyond the end of the section"),
            Error::UnsupportedVersion(v) => write!(f, "unsupported line table version {v}"),
            Error::HeaderLengthExceedsUnit => f.write_str("header length exceeds unit"),
            Error::LineRangeZero => f.write_str("line_range is 0"),
            Error::OpcodeBaseZero => f.write_str("opcode_base is 0"),
            Error::ExtendedOpcodeOverrun(at) => write!(f, "extended opcode at 0x{at:x} overruns the unit"),
            Error::TooManyEntries(n) => write!(f, "entry count {n} exceeds the unit size"),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------------------------

/// Bounds-checked sequential reader over a section, limited to the current unit.
struct Reader<'a> {
    section: &'a [u8],
    /// `section[..end]` of the current unit; every read is checked against it.
    data: &'a [u8],
    pos: usize,
    le: bool,
}

impl<'a> Reader<'a> {
    fn new(section: &'a [u8], le: bool) -> Self {
        Self { section, data: section, pos: 0, le }
    }

    fn set_end(&mut self, end: usize) {
        self.data = self.section.get(..end).unwrap_or(self.section);
    }

    #[inline]
    fn end(&self) -> usize {
        self.data.len()
    }

    #[inline]
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    #[inline]
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let data = self.data;
        let s = self.pos.checked_add(n).and_then(|end| data.get(self.pos..end)).ok_or(Error::UnexpectedEnd(self.pos))?;
        self.pos += n;
        Ok(s)
    }

    #[inline]
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let a = self.data.get(self.pos..).and_then(|s| s.first_chunk::<N>()).ok_or(Error::UnexpectedEnd(self.pos))?;
        self.pos += N;
        Ok(*a)
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        let n = usize::try_from(n).map_err(|_| Error::UnexpectedEnd(self.pos))?;
        self.take(n).map(drop)
    }

    #[inline]
    fn u8(&mut self) -> Result<u8> {
        match self.data.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Err(Error::UnexpectedEnd(self.pos)),
        }
    }

    #[inline]
    fn i8(&mut self) -> Result<i8> {
        self.u8().map(|b| b as i8)
    }

    fn u16(&mut self) -> Result<u16> {
        let a = self.array()?;
        Ok(if self.le { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) })
    }

    fn u32(&mut self) -> Result<u32> {
        let a = self.array()?;
        Ok(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }

    /// Unsigned integer of `n` bytes; wider values keep their low 64 bits.
    fn u_n(&mut self, n: u64) -> Result<u64> {
        let n = usize::try_from(n).map_err(|_| Error::UnexpectedEnd(self.pos))?;
        let bytes = self.take(n)?;
        Ok(if self.le {
            bytes.iter().rev().fold(0, |v, &b| (v << 8) | u64::from(b))
        } else {
            bytes.iter().fold(0, |v, &b| (v << 8) | u64::from(b))
        })
    }

    /// Section offset: 4 bytes in 32-bit DWARF, 8 in 64-bit DWARF.
    fn offset(&mut self, size: usize) -> Result<u64> {
        if size == 4 {
            self.u32().map(u64::from)
        } else {
            self.u_n(8)
        }
    }

    /// ULEB128; bits beyond 64 are dropped.
    #[inline]
    fn uleb(&mut self) -> Result<u64> {
        let mut result = 0u64;
        let mut shift = 0u32;
        loop {
            let b = self.u8()?;
            if shift < 64 {
                result |= u64::from(b & 0x7f) << shift;
            }
            if b & 0x80 == 0 {
                return Ok(result);
            }
            shift = shift.saturating_add(7);
        }
    }

    /// SLEB128; bits beyond 64 are dropped.
    fn sleb(&mut self) -> Result<i64> {
        let mut result = 0i64;
        let mut shift = 0u32;
        loop {
            let b = self.u8()?;
            if shift < 64 {
                result |= i64::from(b & 0x7f) << shift;
            }
            shift = shift.saturating_add(7);
            if b & 0x80 == 0 {
                if shift < 64 && b & 0x40 != 0 {
                    result |= -1i64 << shift;
                }
                return Ok(result);
            }
        }
    }

    /// NUL-terminated string (without the NUL), which must end inside the unit.
    fn cstr(&mut self) -> Result<&'a [u8]> {
        let data = self.data;
        let rest = data.get(self.pos..).unwrap_or_default();
        match rest.iter().position(|&b| b == 0) {
            Some(len) => {
                self.pos += len + 1;
                Ok(&rest[..len])
            }
            None => Err(Error::UnterminatedString(self.pos)),
        }
    }
}

fn string_at(section: Option<&[u8]>, offset: u64, which: StrSection) -> Result<&[u8]> {
    let s = section.ok_or(Error::MissingStrSection(which))?;
    let rest = usize::try_from(offset).ok().and_then(|o| s.get(o..)).filter(|r| !r.is_empty());
    let rest = rest.ok_or(Error::StrOffsetOutside(offset, which))?;
    Ok(rest.iter().position(|&b| b == 0).map_or(rest, |stop| &rest[..stop]))
}

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

#[inline]
fn is_sep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}

fn is_absolute(path: &str) -> bool {
    match path.as_bytes() {
        [c, ..] if is_sep(*c) => true,
        [d, b':', ..] => d.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Normalize `a` (or `a + "/" + b`) : separators become '/', "." segments and duplicate slashes are
/// dropped (".." is kept as-is), a leading "/" or "//" is preserved, and an empty result is ".".
fn normalize(a: &str, b: Option<&str>) -> String {
    let mut head = a.bytes().chain(b.into_iter().flat_map(|s| std::iter::once(b'/').chain(s.bytes())));
    let mut out = String::with_capacity(a.len() + b.map_or(0, |s| s.len() + 1));
    if head.next().is_some_and(is_sep) {
        out.push('/');
        if head.next().is_some_and(is_sep) {
            out.push('/');
        }
    }
    let prefix = out.len();
    let parts = a.split(['/', '\\']).chain(b.into_iter().flat_map(|s| s.split(['/', '\\'])));
    for part in parts {
        if part.is_empty() || part == "." {
            continue;
        }
        if out.len() > prefix {
            out.push('/');
        }
        out.push_str(part);
    }
    if out.is_empty() {
        out.push('.');
    }
    out
}

fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() || is_absolute(name) {
        normalize(name, None)
    } else {
        normalize(dir, Some(name))
    }
}

// ---------------------------------------------------------------------------------------------
// File tables
// ---------------------------------------------------------------------------------------------

/// Interns file paths into one list shared by all units.
#[derive(Default)]
struct FileTable {
    files: Vec<String>,
    index: HashMap<String, u32>,
    unknown: Option<u32>,
}

impl FileTable {
    fn intern(&mut self, path: String) -> u32 {
        match self.index.entry(path) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let i = self.files.len() as u32;
                self.files.push(e.key().clone());
                e.insert(i);
                i
            }
        }
    }

    fn unknown(&mut self) -> u32 {
        match self.unknown {
            Some(i) => i,
            None => {
                let i = self.intern(UNKNOWN_FILE.to_owned());
                self.unknown = Some(i);
                i
            }
        }
    }
}

/// A unit-local file: name plus directory index (joined lazily on first use).
struct FileEntry<'a> {
    dir: usize,
    name: Cow<'a, str>,
}

/// Unit-local directory/file tables and their mapping to global file indices. Reused across units.
#[derive(Default)]
struct UnitFiles<'a> {
    dirs: Vec<Cow<'a, str>>,
    /// `None` = file number with no entry (file 0 before DWARF 5).
    paths: Vec<Option<FileEntry<'a>>>,
    /// Global index per entry of `paths`, `UNRESOLVED` until first use.
    globals: Vec<u32>,
    /// File numbers used while undefined; they stay `<unknown>` even if defined later.
    undefined_used: HashSet<u64>,
}

impl<'a> UnitFiles<'a> {
    fn clear(&mut self) {
        self.dirs.clear();
        self.paths.clear();
        self.globals.clear();
        self.undefined_used.clear();
    }

    fn push(&mut self, entry: Option<FileEntry<'a>>, table: &mut FileTable) {
        let n = self.paths.len() as u64;
        let global =
            if !self.undefined_used.is_empty() && self.undefined_used.contains(&n) { table.unknown() } else { UNRESOLVED };
        self.paths.push(entry);
        self.globals.push(global);
    }

    #[inline]
    fn resolve(&mut self, n: u64, table: &mut FileTable) -> u32 {
        match usize::try_from(n).ok().filter(|&i| i < self.globals.len()) {
            Some(i) => {
                let g = self.globals[i];
                if g != UNRESOLVED {
                    return g;
                }
                let g = match &self.paths[i] {
                    None => table.unknown(),
                    Some(e) => table.intern(join_path(self.dirs.get(e.dir).map_or("", |d| d), &e.name)),
                };
                self.globals[i] = g;
                g
            }
            None => {
                self.undefined_used.insert(n);
                table.unknown()
            }
        }
    }
}

/// Attribute value from a DWARF 5 entry format.
enum Value<'a> {
    Str(&'a [u8]),
    Num(i128),
}

impl<'a> Value<'a> {
    fn into_path(self) -> Cow<'a, str> {
        match self {
            Value::Str(s) => decode_utf8(s),
            Value::Num(n) => Cow::Owned(n.to_string()),
        }
    }

    /// Directory index (`usize::MAX` = no such directory).
    fn into_index(self) -> usize {
        match self {
            Value::Num(n) => usize::try_from(n).unwrap_or(usize::MAX),
            Value::Str(s) => {
                let text = decode_utf8(s);
                let text = text.trim();
                if text.is_empty() {
                    0
                } else {
                    text.parse::<usize>().unwrap_or(usize::MAX)
                }
            }
        }
    }
}

#[inline]
fn to_index(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// Read one attribute value of a directory/file entry.
fn read_form<'a>(r: &mut Reader<'a>, form: u64, offset_size: usize, opts: &DebugLineOptions<'a>) -> Result<Value<'a>> {
    const EMPTY: Value<'static> = Value::Str(b"");
    Ok(match form {
        FORM_STRING => Value::Str(r.cstr()?),
        FORM_LINE_STRP => {
            let off = r.offset(offset_size)?;
            Value::Str(string_at(opts.debug_line_str, off, StrSection::LineStr)?)
        }
        FORM_STRP => {
            let off = r.offset(offset_size)?;
            Value::Str(string_at(opts.debug_str, off, StrSection::Str)?)
        }
        FORM_STRP_SUP | FORM_GNU_STRP_ALT => {
            r.skip(offset_size as u64)?; // supplementary-file strings are not available
            EMPTY
        }
        FORM_STRX | FORM_GNU_STR_INDEX => {
            r.uleb()?; // needs .debug_str_offsets + DW_AT_str_offsets_base from .debug_info
            EMPTY
        }
        FORM_STRX1 | FORM_STRX2 | FORM_STRX3 | FORM_STRX4 => {
            r.skip(form - FORM_STRX1 + 1)?;
            EMPTY
        }
        FORM_UDATA => Value::Num(r.uleb()?.into()),
        FORM_SDATA => Value::Num(r.sleb()?.into()),
        FORM_DATA1 | FORM_FLAG => Value::Num(r.u8()?.into()),
        FORM_DATA2 => Value::Num(r.u16()?.into()),
        FORM_DATA4 => Value::Num(r.u32()?.into()),
        FORM_DATA8 => Value::Num(r.u_n(8)?.into()),
        FORM_SEC_OFFSET => Value::Num(r.offset(offset_size)?.into()),
        FORM_FLAG_PRESENT => Value::Num(1),
        FORM_DATA16 => {
            r.skip(16)?; // e.g. DW_LNCT_MD5
            Value::Num(0)
        }
        FORM_BLOCK => {
            let n = r.uleb()?;
            r.skip(n)?;
            Value::Num(0)
        }
        FORM_BLOCK1 => {
            let n = r.u8()?;
            r.skip(n.into())?;
            Value::Num(0)
        }
        FORM_BLOCK2 => {
            let n = r.u16()?;
            r.skip(n.into())?;
            Value::Num(0)
        }
        FORM_BLOCK4 => {
            let n = r.u32()?;
            r.skip(n.into())?;
            Value::Num(0)
        }
        _ => return Err(Error::UnsupportedForm(form)),
    })
}

/// DWARF 5 directory or file table: calls `sink(path, directory index)` per entry.
fn read_entry_table<'a>(
    r: &mut Reader<'a>,
    offset_size: usize,
    opts: &DebugLineOptions<'a>,
    format: &mut Vec<(u64, u64)>,
    mut sink: impl FnMut(Cow<'a, str>, usize),
) -> Result<()> {
    // Entry format: pairs of (content type, form).
    format.clear();
    let pairs = r.u8()?;
    for _ in 0..pairs {
        let content = r.uleb()?;
        let form = r.uleb()?;
        format.push((content, form));
    }
    let count = r.uleb()?;
    // Entries made only of zero-width forms consume no bytes; refuse absurd counts of them
    // instead of looping (every other entry encoding runs out of data on its own).
    if count > r.remaining() as u64 && format.iter().all(|&(_, form)| form == FORM_FLAG_PRESENT) {
        return Err(Error::TooManyEntries(count));
    }
    for _ in 0..count {
        let mut path = Cow::Borrowed("");
        let mut dir = 0usize;
        for &(content, form) in format.iter() {
            let value = read_form(r, form, offset_size, opts)?;
            match content {
                LNCT_PATH => path = value.into_path(),
                LNCT_DIRECTORY_INDEX => dir = value.into_index(),
                _ => {}
            }
        }
        sink(path, dir);
    }
    Ok(())
}

/// Buffers reused across units.
#[derive(Default)]
struct Scratch<'a> {
    files: UnitFiles<'a>,
    format: Vec<(u64, u64)>,
}

// ---------------------------------------------------------------------------------------------
// Line program
// ---------------------------------------------------------------------------------------------

/// Decode every line-number program in a `.debug_line` section. Rows are returned in program order
/// (one row per emitted matrix row, including end_sequence rows). Sequences whose start address is
/// a linker tombstone (all ones / all ones minus one, used for discarded code) are dropped.
pub fn parse_debug_line(section: &[u8], opts: &DebugLineOptions<'_>) -> DebugLineResult {
    let mut table = FileTable::default();
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut scratch = Scratch::default();
    let mut r = Reader::new(section, opts.little_endian);

    while r.pos < section.len() {
        let unit_start = r.pos;
        r.set_end(section.len());
        let header = (|| -> Result<Option<(usize, usize)>> {
            let mut unit_length = u64::from(r.u32()?);
            let mut offset_size = 4;
            if unit_length == 0xffff_ffff {
                unit_length = r.u_n(8)?;
                offset_size = 8;
            } else if unit_length >= 0xffff_fff0 {
                return Err(Error::ReservedUnitLength(unit_length));
            }
            if unit_length == 0 {
                return Ok(None); // padding
            }
            let unit_end = usize::try_from(unit_length)
                .ok()
                .and_then(|len| r.pos.checked_add(len))
                .filter(|&end| end <= section.len())
                .ok_or(Error::UnitBeyondSection)?;
            Ok(Some((unit_end, offset_size)))
        })();

        match header {
            Err(e) => {
                errors.push(format!("line table at 0x{unit_start:x}: {e}"));
                break; // the next unit's position is unknown
            }
            Ok(None) => {}
            Ok(Some((unit_end, offset_size))) => {
                r.set_end(unit_end);
                if let Err(e) = decode_unit(&mut r, offset_size, opts, &mut table, &mut rows, &mut scratch) {
                    errors.push(format!("line table at 0x{unit_start:x}: {e}"));
                }
                r.pos = unit_end;
            }
        }
    }

    DebugLineResult { files: table.files, rows, errors }
}

fn decode_unit<'a>(
    r: &mut Reader<'a>,
    offset_size: usize,
    opts: &DebugLineOptions<'a>,
    table: &mut FileTable,
    rows: &mut Vec<DebugLineRow>,
    scratch: &mut Scratch<'a>,
) -> Result<()> {
    let unit_end = r.end();
    let version = r.u16()?;
    if !(2..=5).contains(&version) {
        return Err(Error::UnsupportedVersion(version));
    }

    let mut address_size = opts.address_size;
    if version >= 5 {
        address_size = r.u8()?;
        r.u8()?; // segment_selector_size
    }
    let header_length = r.offset(offset_size)?;
    let program_start = usize::try_from(header_length)
        .ok()
        .and_then(|len| r.pos.checked_add(len))
        .filter(|&start| start <= unit_end)
        .ok_or(Error::HeaderLengthExceedsUnit)?;

    let min_inst_length = u64::from(r.u8()?);
    let max_ops = if version >= 4 { u64::from(r.u8()?.max(1)) } else { 1 };
    let default_is_stmt = r.u8()? != 0;
    let line_base = i64::from(r.i8()?);
    let line_range = r.u8()?;
    let opcode_base = r.u8()?;
    if line_range == 0 {
        return Err(Error::LineRangeZero);
    }
    if opcode_base == 0 {
        return Err(Error::OpcodeBaseZero);
    }
    let mut arg_counts = [0u8; 256];
    for count in &mut arg_counts[1..usize::from(opcode_base)] {
        *count = r.u8()?;
    }

    // Unit-local file numbers -> path (lazily interned into the global table on first use).
    let Scratch { files, format } = scratch;
    files.clear();
    if version >= 5 {
        // Directory 0 is the compilation directory; relative entries are relative to it.
        read_entry_table(r, offset_size, opts, format, |path, _| files.dirs.push(path))?;
        if let Some((first, rest)) = files.dirs.split_first_mut() {
            for dir in rest {
                *dir = Cow::Owned(join_path(first, dir));
            }
        }
        read_entry_table(r, offset_size, opts, format, |name, dir| files.push(Some(FileEntry { dir, name }), table))?;
    } else {
        // Directory 0 is the (unrecorded) compilation directory; file numbers start at 1.
        files.dirs.push(Cow::Borrowed(""));
        loop {
            let dir = r.cstr()?;
            if strip_bom(dir).is_empty() {
                break;
            }
            files.dirs.push(decode_utf8(dir));
        }
        files.push(None, table);
        loop {
            let name = r.cstr()?;
            if strip_bom(name).is_empty() {
                break;
            }
            let dir = r.uleb()?;
            r.uleb()?; // modification time
            r.uleb()?; // file length
            files.push(Some(FileEntry { dir: to_index(dir), name: decode_utf8(name) }), table);
        }
    }

    let tombstone = match address_size {
        1..=7 => Some((1u64 << (8 * u32::from(address_size))) - 1),
        8 => Some(u64::MAX),
        _ => None,
    };
    let const_add_pc_ops = u64::from((255 - opcode_base) / line_range);

    // State machine registers (column, discriminator, ISA and block flags are not needed).
    let mut address: u64 = 0;
    let mut op_index: u64 = 0;
    let mut file: u64 = 1;
    let mut line: i64 = 1;
    let mut is_stmt = default_is_stmt;
    let mut discard = false; // current sequence belongs to discarded (tombstoned) code

    let advance = |address: &mut u64, op_index: &mut u64, ops: u64| {
        if max_ops == 1 {
            *address = address.wrapping_add(min_inst_length.wrapping_mul(ops));
        } else {
            let t = op_index.wrapping_add(ops);
            *address = address.wrapping_add(min_inst_length.wrapping_mul(t / max_ops));
            *op_index = t % max_ops;
        }
    };
    macro_rules! emit {
        ($end_sequence:expr) => {
            if !discard {
                let file = files.resolve(file, table);
                rows.push(DebugLineRow { address, file, line, is_stmt, end_sequence: $end_sequence });
            }
        };
    }

    r.pos = program_start;
    while r.pos < unit_end {
        let op = r.u8()?;

        if op >= opcode_base {
            // Special opcode: advance address and line, then append a row.
            let adjusted = op - opcode_base;
            advance(&mut address, &mut op_index, u64::from(adjusted / line_range));
            line = line.wrapping_add(line_base + i64::from(adjusted % line_range));
            emit!(false);
            continue;
        }

        if op == 0 {
            let len = r.uleb()?;
            if len == 0 {
                continue;
            }
            let next = usize::try_from(len)
                .ok()
                .and_then(|len| r.pos.checked_add(len))
                .filter(|&next| next <= unit_end)
                .ok_or(Error::ExtendedOpcodeOverrun(r.pos))?;
            match r.u8()? {
                LNE_END_SEQUENCE => {
                    emit!(true);
                    address = 0;
                    op_index = 0;
                    file = 1;
                    line = 1;
                    is_stmt = default_is_stmt;
                    discard = false;
                }
                LNE_SET_ADDRESS => {
                    address = r.u_n(len - 1)?;
                    op_index = 0;
                    discard = tombstone.is_some_and(|t| address == t || address == t - 1);
                }
                LNE_DEFINE_FILE => {
                    let name = r.cstr()?;
                    let dir = r.uleb()?;
                    files.push(Some(FileEntry { dir: to_index(dir), name: decode_utf8(name) }), table);
                }
                _ => {} // DW_LNE_set_discriminator / unknown: skipped by length below
            }
            r.pos = next;
            continue;
        }

        match op {
            LNS_COPY => emit!(false),
            LNS_ADVANCE_PC => {
                let ops = r.uleb()?;
                advance(&mut address, &mut op_index, ops);
            }
            LNS_ADVANCE_LINE => line = line.wrapping_add(r.sleb()?),
            LNS_SET_FILE => file = r.uleb()?,
            LNS_SET_COLUMN | LNS_SET_ISA => {
                r.uleb()?;
            }
            LNS_NEGATE_STMT => is_stmt = !is_stmt,
            LNS_SET_BASIC_BLOCK | LNS_SET_PROLOGUE_END | LNS_SET_EPILOGUE_BEGIN => {}
            LNS_CONST_ADD_PC => advance(&mut address, &mut op_index, const_add_pc_ops),
            LNS_FIXED_ADVANCE_PC => {
                address = address.wrapping_add(u64::from(r.u16()?));
                op_index = 0;
            }
            _ => {
                // Opcode unknown to us but declared by the header: skip its ULEB128 operands.
                for _ in 0..arg_counts[usize::from(op)] {
                    r.uleb()?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_normalization() {
        assert_eq!(normalize("", None), ".");
        assert_eq!(normalize("/", None), "/");
        assert_eq!(normalize("a/./b//c/", None), "a/b/c");
        assert_eq!(normalize("\\\\srv\\share\\x.c", None), "//srv/share/x.c");
        assert_eq!(normalize("../x", None), "../x");
        assert_eq!(join_path("C:\\proj\\src", "lib.c"), "C:/proj/src/lib.c");
        assert_eq!(join_path("src", "/abs/x.c"), "/abs/x.c");
        assert_eq!(join_path("src", "d:x.c"), "d:x.c");
        assert_eq!(join_path("", "./a.c"), "a.c");
        // The joiner counts towards a leading "//" exactly like string concatenation would.
        assert_eq!(join_path("/", "a.c"), "//a.c");
    }

    #[test]
    fn leb128() {
        let mut r = Reader::new(&[0xe5, 0x8e, 0x26, 0x7f, 0x80, 0x7f, 0x02], true);
        assert_eq!(r.uleb().unwrap(), 624_485);
        assert_eq!(r.sleb().unwrap(), -1);
        assert_eq!(r.sleb().unwrap(), -128);
        assert_eq!(r.sleb().unwrap(), 2);
        assert!(r.uleb().is_err());
        // Over-long encodings keep the low 64 bits instead of overflowing.
        let long = [0xffu8; 20].iter().copied().chain([0x01]).collect::<Vec<_>>();
        assert_eq!(Reader::new(&long, true).uleb().unwrap(), u64::MAX);
    }
}
