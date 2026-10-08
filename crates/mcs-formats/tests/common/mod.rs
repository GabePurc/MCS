//! In-memory ELF / DWARF synthesis helpers shared by the integration tests (port of the helpers in
//! the TypeScript `tests/formats/elf.test.ts`).
#![allow(dead_code)]

use mcs_core::program::{LoadedProgram, ProgramSymbol, Severity, SymbolKind, SymbolSpace};

pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_NOTE: u32 = 7;
pub const SHT_NOBITS: u32 = 8;
pub const SHF_WRITE: u64 = 1;
pub const SHF_ALLOC: u64 = 2;
pub const SHF_EXECINSTR: u64 = 4;
pub const SHN_ABS: u16 = 0xfff1;
pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;
pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;

/// Append-only byte writer with selectable byte order.
pub struct Writer {
    buf: Vec<u8>,
    le: bool,
}

impl Writer {
    pub fn new(le: bool) -> Self {
        Self { buf: Vec::new(), le }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Raw bytes; values are truncated to 8 bits (so -5 writes 0xfb).
    pub fn u8(&mut self, values: &[i64]) -> &mut Self {
        self.buf.extend(values.iter().map(|&v| v as u8));
        self
    }

    fn ordered(&mut self, le_bytes: &[u8]) -> &mut Self {
        if self.le {
            self.buf.extend_from_slice(le_bytes);
        } else {
            self.buf.extend(le_bytes.iter().rev());
        }
        self
    }

    pub fn u16(&mut self, v: u64) -> &mut Self {
        self.ordered(&(v as u16).to_le_bytes())
    }

    pub fn u32(&mut self, v: u64) -> &mut Self {
        self.ordered(&(v as u32).to_le_bytes())
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.ordered(&v.to_le_bytes())
    }

    pub fn word(&mut self, is64: bool, v: u64) -> &mut Self {
        if is64 {
            self.u64(v)
        } else {
            self.u32(v)
        }
    }

    pub fn uleb(&mut self, mut v: u64) -> &mut Self {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            self.buf.push(b);
            if v == 0 {
                return self;
            }
        }
    }

    pub fn sleb(&mut self, mut v: i64) -> &mut Self {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            let done = (v == 0 && b & 0x40 == 0) || (v == -1 && b & 0x40 != 0);
            self.buf.push(if done { b } else { b | 0x80 });
            if done {
                return self;
            }
        }
    }

    pub fn str(&mut self, s: &str) -> &mut Self {
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        self
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    pub fn pad(&mut self, align: usize) -> &mut Self {
        while !self.buf.len().is_multiple_of(align) {
            self.buf.push(0);
        }
        self
    }

    pub fn patch(&mut self, at: usize, size: usize, v: u64) {
        let mut tmp = Writer::new(self.le);
        tmp.word(size == 8, v);
        self.buf[at..at + size].copy_from_slice(&tmp.buf);
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.buf.clone()
    }
}

#[derive(Clone, Default)]
pub struct TestSection {
    pub name: &'static str,
    pub sh_type: u32,
    pub flags: u64,
    pub addr: u64,
    pub data: Vec<u8>,
    /// Size of SHT_NOBITS sections.
    pub size: u64,
    pub link: u32,
    pub entsize: u64,
}

pub fn section(name: &'static str, sh_type: u32, flags: u64, addr: u64, data: &[u8]) -> TestSection {
    TestSection { name, sh_type, flags, addr, data: data.to_vec(), ..Default::default() }
}

#[derive(Clone, Copy)]
pub struct TestSegment {
    /// ELF section index (1-based, as in the section header table) providing the bytes.
    pub section: usize,
    pub vaddr: u64,
    pub paddr: u64,
}

pub struct ElfSpec {
    pub is64: bool,
    pub le: bool,
    pub e_type: u16,
    pub machine: u16,
    pub entry: u64,
    pub sections: Vec<TestSection>,
    pub segments: Vec<TestSegment>,
}

impl ElfSpec {
    /// Little-endian ELF32 AVR executable.
    pub fn new(sections: Vec<TestSection>) -> Self {
        Self { is64: false, le: true, e_type: 2, machine: 83, entry: 0, sections, segments: Vec::new() }
    }
}

/// Lay out a complete ELF image: header, program headers, section data, section headers.
pub fn build_elf(spec: &ElfSpec) -> Vec<u8> {
    let is64 = spec.is64;
    let le = spec.le;
    let ehsize: u64 = if is64 { 64 } else { 52 };
    let phentsize: u64 = if is64 { 56 } else { 32 };
    let shentsize: u64 = if is64 { 64 } else { 40 };
    let segments = &spec.segments;
    let mut all = vec![TestSection::default()];
    all.extend(spec.sections.iter().cloned());
    all.push(TestSection { name: ".shstrtab", sh_type: SHT_STRTAB, ..Default::default() });

    let mut shstr = Writer::new(le);
    shstr.u8(&[0]);
    let name_offsets: Vec<u64> = all
        .iter()
        .map(|s| {
            if s.name.is_empty() {
                return 0;
            }
            let off = shstr.len() as u64;
            shstr.str(s.name);
            off
        })
        .collect();
    all.last_mut().unwrap().data = shstr.bytes();

    let has_bytes = |s: &TestSection| s.sh_type != SHT_NOBITS && !s.data.is_empty();
    let align8 = |n: u64| (n + 7) & !7;
    let mut cursor = ehsize + segments.len() as u64 * phentsize;
    let offsets: Vec<u64> = all
        .iter()
        .map(|s| {
            if !has_bytes(s) {
                return if s.sh_type == SHT_NOBITS { cursor } else { 0 };
            }
            cursor = align8(cursor);
            let off = cursor;
            cursor += s.data.len() as u64;
            off
        })
        .collect();
    let shoff = align8(cursor);
    let size_of = |s: &TestSection| if s.sh_type == SHT_NOBITS { s.size } else { s.data.len() as u64 };

    let mut w = Writer::new(le);
    w.u8(&[0x7f, 0x45, 0x4c, 0x46, if is64 { 2 } else { 1 }, if le { 1 } else { 2 }, 1]).pad(16);
    w.u16(spec.e_type.into()).u16(spec.machine.into()).u32(1);
    w.word(is64, spec.entry).word(is64, if segments.is_empty() { 0 } else { ehsize }).word(is64, shoff);
    w.u32(0).u16(ehsize).u16(phentsize).u16(segments.len() as u64).u16(shentsize);
    w.u16(all.len() as u64).u16(all.len() as u64 - 1);

    for seg in segments {
        let s = &all[seg.section];
        let filesz = if s.sh_type == SHT_NOBITS { 0 } else { size_of(s) };
        let memsz = size_of(s);
        let off = offsets[seg.section];
        if is64 {
            w.u32(1).u32(5).u64(off).u64(seg.vaddr).u64(seg.paddr).u64(filesz).u64(memsz).u64(1);
        } else {
            w.u32(1).u32(off).u32(seg.vaddr).u32(seg.paddr).u32(filesz).u32(memsz).u32(5).u32(1);
        }
    }

    for s in &all {
        if has_bytes(s) {
            w.pad(8).raw(&s.data);
        }
    }
    w.pad(8);
    assert_eq!(w.len() as u64, shoff);

    for (i, s) in all.iter().enumerate() {
        let size = size_of(s);
        let link = u64::from(s.link);
        if is64 {
            w.u32(name_offsets[i]).u32(s.sh_type.into()).u64(s.flags).u64(s.addr).u64(offsets[i]).u64(size);
            w.u32(link).u32(0).u64(1).u64(s.entsize);
        } else {
            w.u32(name_offsets[i]).u32(s.sh_type.into()).u32(s.flags).u32(s.addr).u32(offsets[i]).u32(size);
            w.u32(link).u32(0).u32(1).u32(s.entsize);
        }
    }
    w.bytes()
}

pub struct TestSymbol {
    pub name: &'static str,
    pub value: u64,
    pub size: u64,
    pub sym_type: u8,
    pub bind: u8,
    pub shndx: u16,
}

pub const fn tsym(name: &'static str, value: u64, size: u64, sym_type: u8, bind: u8, shndx: u16) -> TestSymbol {
    TestSymbol { name, value, size, sym_type, bind, shndx }
}

/// Returns (symtab, strtab).
pub fn build_symtab(symbols: &[TestSymbol], is64: bool, le: bool) -> (Vec<u8>, Vec<u8>) {
    let mut strs = Writer::new(le);
    strs.u8(&[0]);
    let mut tab = Writer::new(le);
    let entry = |tab: &mut Writer, name: u64, value: u64, size: u64, info: u8, shndx: u16| {
        if is64 {
            tab.u32(name).u8(&[info.into(), 0]).u16(shndx.into()).u64(value).u64(size);
        } else {
            tab.u32(name).u32(value).u32(size).u8(&[info.into(), 0]).u16(shndx.into());
        }
    };
    entry(&mut tab, 0, 0, 0, 0, 0);
    for s in symbols {
        let name = if s.name.is_empty() { 0 } else { strs.len() as u64 };
        if !s.name.is_empty() {
            strs.str(s.name);
        }
        entry(&mut tab, name, s.value, s.size, (s.bind << 4) | s.sym_type, s.shndx);
    }
    (tab.bytes(), strs.bytes())
}

/// Extended opcode: 0, ULEB length, sub-opcode, payload.
pub fn ext(w: &mut Writer, sub: u8, le: bool, payload: impl FnOnce(&mut Writer)) {
    let mut p = Writer::new(le);
    payload(&mut p);
    w.u8(&[0]).uleb(p.len() as u64 + 1).u8(&[sub.into()]).raw(&p.bytes());
}

/// Hand-assembled `.debug_line` with three sequences in two units:
///  - unit 1: DWARF 2, 32-bit, min_inst_length 2, opcode_base 10 (so opcode 10 is special), with
///    include dirs, define_file, const_add_pc, fixed_advance_pc and set_discriminator;
///  - unit 2: DWARF 4, 64-bit DWARF, default_is_stmt 0, opcode_base 14 (opcode 13 is unknown with
///    two operands), backslash/absolute paths, an unknown extended opcode, a duplicate row and a
///    sequence at a tombstone address (discarded code).
pub fn build_debug_line() -> Vec<u8> {
    let mut w = Writer::new(true);

    // ---- unit 1 (DWARF 2) ----
    let u1 = w.len();
    w.u32(0).u16(2);
    let hl1 = w.len();
    w.u32(0);
    let h1 = w.len();
    w.u8(&[2, 1, -5, 14, 10]); // min_inst_length, default_is_stmt, line_base, line_range, opcode_base
    w.u8(&[0, 1, 1, 1, 1, 0, 0, 0, 1]);
    w.str("src").u8(&[0]);
    w.str("main.c").uleb(1).uleb(0).uleb(0);
    w.str("util.h").uleb(0).uleb(0).uleb(0);
    w.u8(&[0]);
    let v = (w.len() - h1) as u64;
    w.patch(hl1, 4, v);
    let sp1 = |ops: i64, d_line: i64| d_line + 5 + 14 * ops + 10;

    ext(&mut w, 2, true, |p| {
        p.u32(0x10); // set_address 0x10
    });
    w.u8(&[3]).sleb(9); // line 10
    w.u8(&[1]); // copy                       -> 0x10 main.c:10
    w.u8(&[sp1(1, 1)]); //                    -> 0x12 main.c:11
    w.u8(&[6]); // negate_stmt
    w.u8(&[sp1(2, 0)]); //                    -> 0x16 main.c:11 (not stmt)
    w.u8(&[6]);
    w.u8(&[4]).uleb(2); // util.h
    w.u8(&[3]).sleb(-8); // line 3
    w.u8(&[2]).uleb(1); // advance_pc 1 op = 2 bytes
    w.u8(&[1]); //                            -> 0x18 util.h:3
    ext(&mut w, 3, true, |p| {
        p.str("gen.c").uleb(1).uleb(0).uleb(0); // define_file #3 = src/gen.c
    });
    w.u8(&[4]).uleb(3);
    w.u8(&[8]); // const_add_pc: (255-10)/14 = 17 ops = 34 bytes -> 0x3a
    w.u8(&[9]).u16(2); // fixed_advance_pc -> 0x3c
    w.u8(&[3]).sleb(17); // line 20
    w.u8(&[1]); //                            -> 0x3c gen.c:20
    w.u8(&[10]); // special (adjusted 0): line -5 -> 0x3c gen.c:15
    ext(&mut w, 4, true, |p| {
        p.uleb(3); // set_discriminator
    });
    w.u8(&[2]).uleb(1);
    ext(&mut w, 1, true, |_| {}); // end_sequence -> 0x3e
    let v = (w.len() - u1 - 4) as u64;
    w.patch(u1, 4, v);

    // ---- unit 2 (DWARF 4, 64-bit DWARF) ----
    w.u32(0xffff_ffff);
    let u2 = w.len();
    w.u64(0).u16(4);
    let hl2 = w.len();
    w.u64(0);
    let h2 = w.len();
    w.u8(&[2, 1, 0, -3, 12, 14]); // min_inst, max_ops, default_is_stmt=0, line_base, line_range, opcode_base
    w.u8(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1, 2]);
    w.str("C:\\proj\\src").u8(&[0]);
    w.str("src/main.c").uleb(0).uleb(0).uleb(0);
    w.str("lib.c").uleb(1).uleb(0).uleb(0);
    w.str("/abs/x.c").uleb(1).uleb(0).uleb(0);
    w.u8(&[0]);
    let v = (w.len() - h2) as u64;
    w.patch(hl2, 8, v);
    let sp2 = |ops: i64, d_line: i64| d_line + 3 + 12 * ops + 14;

    ext(&mut w, 2, true, |p| {
        p.u32(0x40);
    });
    w.u8(&[4]).uleb(2); // lib.c
    w.u8(&[3]).sleb(4); // line 5
    w.u8(&[10, 5, 3, 7, 11, 12, 0]); // prologue_end, set_column 3, basic_block, epilogue_begin, set_isa 0
    w.u8(&[13]).uleb(300).uleb(5); // unknown standard opcode with 2 operands
    w.u8(&[1]); //                            -> 0x40 lib.c:5
    ext(&mut w, 0x80, true, |p| {
        p.u8(&[1, 2, 3]); // unknown extended opcode
    });
    w.u8(&[4]).uleb(1); // src/main.c
    w.u8(&[3]).sleb(1);
    w.u8(&[sp2(1, 0)]); //                    -> 0x42 main.c:6
    w.u8(&[1]); //                            -> 0x42 main.c:6 (duplicate)
    w.u8(&[4]).uleb(3); // /abs/x.c
    w.u8(&[sp2(0, 1)]); //                    -> 0x42 x.c:7
    w.u8(&[2]).uleb(1);
    ext(&mut w, 1, true, |_| {}); //          -> 0x44 end
    ext(&mut w, 2, true, |p| {
        p.u32(0xffff_ffff); // discarded sequence
    });
    w.u8(&[1]).u8(&[2]).uleb(2);
    ext(&mut w, 1, true, |_| {});
    let v = (w.len() - u2 - 8) as u64;
    w.patch(u2, 8, v);

    w.bytes()
}

pub fn build_avr_device_note(name: &str) -> Vec<u8> {
    let mut desc = Writer::new(true);
    desc.u32(0) // flash start
        .u32(1024) // flash size
        .u32(0x40) // SRAM start
        .u32(32) // SRAM size
        .u32(0) // EEPROM start
        .u32(0) // EEPROM size
        .u32(8) // offset table length
        .u32(1) // device name offset in the string table
        .u8(&[0])
        .str(name);
    let mut note = Writer::new(true);
    note.u32(4).u32(desc.len() as u64).u32(1).str("AVR").raw(&desc.bytes()).pad(4);
    note.bytes()
}

pub fn text_bytes() -> Vec<u8> {
    (1..=0x20).collect()
}

/// A linked-looking ATtiny program: text, initialized data, bss, eeprom, fuse, lock, signature.
pub fn build_avr_elf(debug_line: &[u8]) -> Vec<u8> {
    let (symtab, strtab) = build_symtab(
        &[
            tsym("main.c", 0, 0, STT_FILE, STB_LOCAL, SHN_ABS),
            tsym("", 0, 0, STT_SECTION, STB_LOCAL, 1),
            tsym("__vectors", 0, 0, STT_NOTYPE, STB_GLOBAL, 1),
            tsym("loop", 0x14, 0, STT_NOTYPE, STB_LOCAL, 1),
            tsym("main", 0x10, 0x10, STT_FUNC, STB_GLOBAL, 1),
            tsym("__ctors_end", 0x0e, 0, STT_NOTYPE, STB_GLOBAL, 1),
            tsym("counter", 0x80_0040, 2, STT_OBJECT, STB_GLOBAL, 2),
            tsym("buffer", 0x80_0044, 2, STT_OBJECT, STB_LOCAL, 3),
            tsym("ee_cfg", 0x81_0002, 3, STT_OBJECT, STB_GLOBAL, 4),
            tsym("__fuse", 0x82_0000, 1, STT_OBJECT, STB_GLOBAL, 5),
            tsym("__SP_L__", 0x3d, 0, STT_NOTYPE, STB_LOCAL, SHN_ABS),
            tsym("__stack", 0x80_005f, 0, STT_NOTYPE, STB_GLOBAL, SHN_ABS),
            tsym("F_CPU", 8_000_000, 0, STT_NOTYPE, STB_GLOBAL, SHN_ABS),
            tsym("weak_handler", 0x18, 2, STT_FUNC, STB_WEAK, 1),
            tsym("ext_func", 0, 0, STT_NOTYPE, STB_GLOBAL, 0),
        ],
        false,
        true,
    );
    let mut spec = ElfSpec::new(vec![
        section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 0, &text_bytes()), // 1
        section(".data", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 0x80_0040, &[0xde, 0xad, 0xbe, 0xef]), // 2
        TestSection { name: ".bss", sh_type: SHT_NOBITS, flags: SHF_ALLOC | SHF_WRITE, addr: 0x80_0044, size: 2, ..Default::default() }, // 3
        section(".eeprom", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 0x81_0002, &[1, 2, 3]), // 4
        section(".fuse", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 0x82_0000, &[0xfe]), // 5
        section(".lock", SHT_PROGBITS, SHF_ALLOC | SHF_WRITE, 0x83_0000, &[0xfc]), // 6
        section(".signature", SHT_PROGBITS, SHF_ALLOC, 0x84_0000, &[0x1e, 0x90, 0x03]), // 7
        section(".note.gnu.avr.deviceinfo", SHT_NOTE, 0, 0, &build_avr_device_note("ATtiny10")), // 8
        section(".debug_line", SHT_PROGBITS, 0, 0, debug_line), // 9
        TestSection { name: ".symtab", sh_type: SHT_SYMTAB, data: symtab, link: 11, entsize: 16, ..Default::default() }, // 10
        section(".strtab", SHT_STRTAB, 0, 0, &strtab), // 11
    ]);
    spec.segments = vec![
        TestSegment { section: 1, vaddr: 0, paddr: 0 },
        TestSegment { section: 2, vaddr: 0x80_0040, paddr: 0x20 }, // .data: VMA in SRAM, LMA right after .text
        TestSegment { section: 3, vaddr: 0x80_0044, paddr: 0x24 },
        TestSegment { section: 4, vaddr: 0x81_0002, paddr: 0x81_0002 },
        TestSegment { section: 5, vaddr: 0x82_0000, paddr: 0x82_0000 },
        TestSegment { section: 6, vaddr: 0x83_0000, paddr: 0x83_0000 },
        TestSegment { section: 7, vaddr: 0x84_0000, paddr: 0x84_0000 },
    ];
    build_elf(&spec)
}

/// Big-endian ELF64 for a non-AVR machine with a DWARF 3 line program (8-byte set_address).
pub fn build_be64_elf() -> Vec<u8> {
    let (symtab, strtab) = build_symtab(
        &[tsym("start", 0x100, 4, STT_FUNC, STB_GLOBAL, 1), tsym("table", 0x102, 2, STT_OBJECT, STB_LOCAL, 1)],
        true,
        false,
    );
    let mut dl = Writer::new(false);
    dl.u32(0).u16(3);
    dl.u32(0);
    let h = dl.len();
    dl.u8(&[1, 1, -5, 14, 10]).u8(&[0, 1, 1, 1, 1, 0, 0, 0, 1]).u8(&[0]).str("start.s").uleb(0).uleb(0).uleb(0).u8(&[0]);
    let v = (dl.len() - h) as u64;
    dl.patch(6, 4, v);
    ext(&mut dl, 2, false, |p| {
        p.u64(0x100);
    });
    dl.u8(&[3]).sleb(2).u8(&[1]).u8(&[2]).uleb(4);
    ext(&mut dl, 1, false, |_| {});
    let v = (dl.len() - 4) as u64;
    dl.patch(0, 4, v);

    let mut spec = ElfSpec::new(vec![
        section(".text", SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR, 0x100, &[9, 8, 7, 6]),
        section(".debug_line", SHT_PROGBITS, 0, 0, &dl.bytes()),
        TestSection { name: ".symtab", sh_type: SHT_SYMTAB, data: symtab, link: 4, entsize: 24, ..Default::default() },
        section(".strtab", SHT_STRTAB, 0, 0, &strtab),
    ]);
    spec.is64 = true;
    spec.le = false;
    spec.machine = 2;
    spec.entry = 0x100;
    spec.segments = vec![TestSegment { section: 1, vaddr: 0x100, paddr: 0x100 }];
    build_elf(&spec)
}

pub fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Contents of a named section of a little-endian ELF32 file (test-side reader, independent of the
/// crate's own parser).
pub fn elf32_section<'a>(elf: &'a [u8], name: &str) -> &'a [u8] {
    let u16_at = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
    let u32_at = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap()) as usize;
    let shoff = u32_at(32);
    let (shentsize, shnum, shstrndx) = (u16_at(46), u16_at(48), u16_at(50));
    let header = |i: usize| shoff + i * shentsize;
    let strtab = u32_at(header(shstrndx) + 16);
    (0..shnum)
        .find_map(|i| {
            let h = header(i);
            let name_at = strtab + u32_at(h);
            let end = name_at + elf[name_at..].iter().position(|&b| b == 0).unwrap();
            (&elf[name_at..end] == name.as_bytes()).then(|| {
                let (off, size) = (u32_at(h + 16), u32_at(h + 20));
                &elf[off..off + size]
            })
        })
        .unwrap_or_else(|| panic!("section {name} not found"))
}

pub fn severities(p: &LoadedProgram) -> Vec<Severity> {
    p.diagnostics.iter().map(|d| d.severity).collect()
}

pub fn psym(name: &str, address: u32, size: u32, kind: SymbolKind, space: SymbolSpace, global: bool) -> ProgramSymbol {
    ProgramSymbol { name: name.to_owned(), address, size, kind, space, global }
}

/// Deterministic xorshift PRNG for the fuzz-style tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
