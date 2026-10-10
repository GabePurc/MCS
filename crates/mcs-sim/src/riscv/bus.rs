//! RV32 memory system: windows onto backing memories, memory-mapped peripherals and the
//! pre-decoded instruction cache.
//!
//! The address space is divided into 4 KiB pages; a flat table (one byte per page, 1 MiB) maps
//! every page to a *region* (a window onto a backing memory or a peripheral), so any access costs
//! one table lookup plus a bounds check no matter how many regions a device has. Several windows
//! may alias one backing memory (the ESP32-C3 sees its SRAM both as IRAM and as DRAM), each with
//! its own read / write / execute permissions. Windows must be 4 KiB aligned and sized; at most
//! 255 regions exist.
//!
//! Code is pre-decoded lazily: every 4 KiB page of a backing memory that is executed from gets a
//! table of [`Insn`] (one slot per halfword) that is filled on first fetch. Any write to the memory
//! (store, [`Bus::load`], a peripheral's DMA through [`Bus::write_bytes`]) clears the slots it
//! touches, so self-modifying code and code loaded into IRAM at run time stay coherent without an
//! explicit `fence.i`.
//!
//! The ESP32-C3 address map used by [`Bus::esp32c3`] is from the ESP32-C3 Technical Reference
//! Manual, chapter "System and Memory" (internal memory address mapping).

use mcs_core::riscv::{decode, Insn, Op};

/// Region permissions.
pub const PERM_R: u8 = 1;
pub const PERM_W: u8 = 2;
pub const PERM_X: u8 = 4;
pub const PERM_RW: u8 = PERM_R | PERM_W;
pub const PERM_RX: u8 = PERM_R | PERM_X;
pub const PERM_RWX: u8 = PERM_R | PERM_W | PERM_X;

const PAGE_SHIFT: u32 = 12;
const PAGE_SIZE: u32 = 1 << PAGE_SHIFT;
const PAGES: usize = 1 << (32 - PAGE_SHIFT);
/// Instruction slots (halfwords) per code page.
const SLOTS: usize = (PAGE_SIZE / 2) as usize;
const NO_PAGE: u32 = u32::MAX;
const KIND_MEM: u8 = 0;
const KIND_DEV: u8 = 1;

/// An access hit unmapped memory or violated the permissions of its region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccessFault;

/// Handle of a backing memory created with [`Bus::add_mem`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemId(pub usize);

/// Services handed to a peripheral during an access.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cx {
    /// CPU cycle counter at the time of the access.
    pub cycles: u64,
    /// Interrupt lines (bit n = line n) the peripheral asserts; applied to `mip` after the access.
    pub irq_raise: u32,
    /// Interrupt lines the peripheral deasserts.
    pub irq_lower: u32,
}

/// A memory-mapped peripheral. Peripherals are event driven (never ticked); a device that needs to
/// change interrupt lines does so through [`Cx`] during an access, or through
/// [`Machine::set_irq`](super::Machine::set_irq) from the event loop that owns the machine.
pub trait Mmio: Send {
    /// Reads `size` (1, 2 or 4) bytes at `offset` from the start of the window.
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32;
    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx);
}

#[inline]
fn size_mask(size: u32) -> u32 {
    match size {
        1 => 0xff,
        2 => 0xffff,
        _ => u32::MAX,
    }
}

#[derive(Clone, Copy)]
struct Region {
    base: u32,
    /// Offset of the window start in the backing memory.
    off: u32,
    /// Backing memory index (`KIND_MEM`) or device index (`KIND_DEV`).
    target: u32,
    kind: u8,
    perm: u8,
}

const NO_REGION: Region = Region { base: 0, off: 0, target: 0, kind: KIND_MEM, perm: 0 };

struct Mem {
    data: Vec<u8>,
    /// Code arena index of every 4 KiB page of `data`, or `NO_PAGE`.
    code_idx: Vec<u32>,
    /// Number of allocated code pages (0 = never executed from, writes skip the invalidation).
    ncode: u32,
}

pub struct Bus {
    /// Region index of every page; 0 = unmapped.
    pages: Vec<u8>,
    regions: Vec<Region>,
    mems: Vec<Mem>,
    devs: Vec<Box<dyn Mmio>>,
    /// Flat arena of pre-decoded pages: page `p` occupies `code[p * SLOTS..(p + 1) * SLOTS]`.
    pub(crate) code: Vec<Insn>,
    pub(crate) cx: Cx,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    /// An empty address space (every access faults until memories are mapped).
    pub fn new() -> Self {
        Self { pages: vec![0; PAGES], regions: vec![NO_REGION], mems: Vec::new(), devs: Vec::new(), code: Vec::new(), cx: Cx::default() }
    }

    /// Allocates a zero-filled backing memory of `size` bytes (rounded up to 4 KiB).
    pub fn add_mem(&mut self, size: u32) -> MemId {
        let size = (size.max(1) + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        self.mems.push(Mem { data: vec![0; size as usize], code_idx: vec![NO_PAGE; (size / PAGE_SIZE) as usize], ncode: 0 });
        MemId(self.mems.len() - 1)
    }

    /// Size in bytes of a backing memory.
    pub fn mem_size(&self, mem: MemId) -> u32 {
        self.mems[mem.0].data.len() as u32
    }

    fn claim(&mut self, base: u32, size: u32, region: Region) -> Result<(), String> {
        if !base.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) || size == 0 {
            return Err(format!("window {base:#010x}+{size:#x} is not 4 KiB aligned"));
        }
        if self.regions.len() >= 255 {
            return Err("too many memory regions".into());
        }
        let first = (base >> PAGE_SHIFT) as usize;
        let n = (size >> PAGE_SHIFT) as usize;
        if first + n > PAGES {
            return Err(format!("window {base:#010x}+{size:#x} wraps the address space"));
        }
        if let Some(p) = self.pages[first..first + n].iter().position(|&c| c != 0) {
            return Err(format!("window overlaps another one at {:#010x}", base + (p as u32) * PAGE_SIZE));
        }
        let idx = self.regions.len() as u8;
        self.regions.push(Region { base, ..region });
        self.pages[first..first + n].fill(idx);
        Ok(())
    }

    /// Maps `size` bytes of backing memory `mem` starting at `mem_off` into the address space at
    /// `base`.
    pub fn map(&mut self, base: u32, size: u32, mem: MemId, mem_off: u32, perm: u8) -> Result<(), String> {
        let len = self.mems[mem.0].data.len() as u64;
        if !mem_off.is_multiple_of(PAGE_SIZE) || mem_off as u64 + size as u64 > len {
            return Err(format!("window {base:#010x}+{size:#x} does not fit memory {} (offset {mem_off:#x})", mem.0));
        }
        self.claim(base, size, Region { base, off: mem_off, target: mem.0 as u32, kind: KIND_MEM, perm })
    }

    /// Maps peripheral `dev` over `[base, base + size)`; returns its index (see [`Bus::device_mut`]).
    pub fn add_device(&mut self, base: u32, size: u32, dev: Box<dyn Mmio>) -> Result<usize, String> {
        let idx = self.devs.len();
        self.claim(base, size, Region { base, off: 0, target: idx as u32, kind: KIND_DEV, perm: PERM_RW })?;
        self.devs.push(dev);
        Ok(idx)
    }

    pub fn device_mut(&mut self, idx: usize) -> Option<&mut (dyn Mmio + 'static)> {
        self.devs.get_mut(idx).map(|d| d.as_mut())
    }

    /// The ESP32-C3 memory map (TRM "System and Memory", internal memory): boot ROM, flash cache
    /// windows (IROM / DROM, `flash_size` bytes, read-only), 400 KiB SRAM seen as IRAM and DRAM,
    /// and the 8 KiB RTC fast memory. Peripherals (0x6000_0000 and up) are added by the caller.
    /// Returns the bus plus the backing memories `(flash, sram)`.
    pub fn esp32c3(flash_size: u32) -> (Bus, MemId, MemId) {
        let mut bus = Bus::new();
        let flash = bus.add_mem(flash_size.clamp(PAGE_SIZE, 8 << 20));
        let sram = bus.add_mem(0x64000);
        let rom = bus.add_mem(0x60000);
        let rtc = bus.add_mem(0x2000);
        let fs = bus.mem_size(flash);
        let ok = "esp32c3 memory map is static";
        bus.map(0x4200_0000, fs, flash, 0, PERM_RX).expect(ok);
        bus.map(0x3c00_0000, fs, flash, 0, PERM_R).expect(ok);
        bus.map(0x4037_c000, 0x64000, sram, 0, PERM_RWX).expect(ok);
        bus.map(0x3fc8_0000, 0x60000, sram, 0x4000, PERM_RW).expect(ok);
        bus.map(0x4000_0000, 0x60000, rom, 0, PERM_RX).expect(ok);
        bus.map(0x3ff0_0000, 0x20000, rom, 0, PERM_R).expect(ok);
        bus.map(0x5000_0000, 0x2000, rtc, 0, PERM_RWX).expect(ok);
        (bus, flash, sram)
    }

    #[inline]
    fn region(&self, addr: u32) -> &Region {
        &self.regions[self.pages[(addr >> PAGE_SHIFT) as usize] as usize]
    }

    /// Reads `size` (1, 2 or 4) bytes, little-endian; `size` must divide `addr` (checked by the CPU).
    #[inline]
    pub fn read(&mut self, addr: u32, size: u32, cycles: u64) -> Result<u32, AccessFault> {
        let r = *self.region(addr);
        if r.perm & PERM_R == 0 {
            return Err(AccessFault);
        }
        if r.kind == KIND_MEM {
            let d = &self.mems[r.target as usize].data;
            let i = (r.off + (addr - r.base)) as usize;
            match size {
                4 => d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                2 => d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32),
                _ => d.get(i).map(|&b| b as u32),
            }
            .ok_or(AccessFault)
        } else {
            Ok(self.dev_read(r.target, addr - r.base, size, cycles))
        }
    }

    #[inline(never)]
    fn dev_read(&mut self, dev: u32, off: u32, size: u32, cycles: u64) -> u32 {
        self.cx.cycles = cycles;
        self.devs[dev as usize].read(off, size as u8, &mut self.cx) & size_mask(size)
    }

    #[inline(never)]
    fn dev_write(&mut self, dev: u32, off: u32, size: u32, value: u32, cycles: u64) {
        self.cx.cycles = cycles;
        self.devs[dev as usize].write(off, size as u8, value & size_mask(size), &mut self.cx);
    }

    /// Writes `size` (1, 2 or 4) bytes, little-endian, invalidating pre-decoded code.
    #[inline]
    pub fn write(&mut self, addr: u32, size: u32, value: u32, cycles: u64) -> Result<(), AccessFault> {
        let r = *self.region(addr);
        if r.perm & PERM_W == 0 {
            return Err(AccessFault);
        }
        if r.kind == KIND_MEM {
            let m = &mut self.mems[r.target as usize];
            let i = (r.off + (addr - r.base)) as usize;
            let b = value.to_le_bytes();
            match m.data.get_mut(i..i + size as usize) {
                Some(d) => d.copy_from_slice(&b[..size as usize]),
                None => return Err(AccessFault),
            }
            if m.ncode != 0 && Self::touches_code(m, i) {
                Self::invalidate(m, &mut self.code, i, size as usize);
            }
            Ok(())
        } else {
            self.dev_write(r.target, addr - r.base, size, value, cycles);
            Ok(())
        }
    }

    /// True if a write at byte offset `off` may hit pre-decoded code: its page has a code table, or
    /// the page before it does and the write is in the first halfword (a 32-bit instruction may
    /// start in the previous page).
    #[inline(always)]
    fn touches_code(m: &Mem, off: usize) -> bool {
        let page = off >> PAGE_SHIFT;
        m.code_idx.get(page).is_some_and(|&c| c != NO_PAGE) || (off & (PAGE_SIZE as usize - 1) < 2 && page > 0 && m.code_idx[page - 1] != NO_PAGE)
    }

    /// Clears the pre-decoded slots overlapping bytes `[off, off + n)` of `m`, including the
    /// instruction that starts in the halfword before `off` and may span into it.
    #[inline(never)]
    fn invalidate(m: &Mem, code: &mut [Insn], off: usize, n: usize) {
        let first = (off >> 1).saturating_sub(1);
        let last = (off + n - 1) >> 1;
        for h in first..=last {
            if let Some(&ci) = m.code_idx.get(h / SLOTS) {
                if ci != NO_PAGE {
                    code[ci as usize * SLOTS + h % SLOTS] = Insn::UNDECODED;
                }
            }
        }
    }

    /// Host-side bulk write (image loading, DMA): ignores permissions, skips peripherals and
    /// unmapped space, invalidates pre-decoded code. Returns the number of bytes written.
    pub fn write_bytes(&mut self, addr: u32, bytes: &[u8]) -> usize {
        let mut done = 0;
        while done < bytes.len() {
            let a = addr.wrapping_add(done as u32);
            let r = *self.region(a);
            let page_left = (PAGE_SIZE - (a & (PAGE_SIZE - 1))) as usize;
            let n = page_left.min(bytes.len() - done);
            if r.perm == 0 || r.kind != KIND_MEM {
                done += n;
                continue;
            }
            let m = &mut self.mems[r.target as usize];
            let i = (r.off + (a - r.base)) as usize;
            if let Some(d) = m.data.get_mut(i..i + n) {
                d.copy_from_slice(&bytes[done..done + n]);
                if m.ncode != 0 && (Self::touches_code(m, i) || Self::touches_code(m, i + n - 1)) {
                    Self::invalidate(m, &mut self.code, i, n);
                }
            }
            done += n;
        }
        bytes.len()
    }

    /// Loads an image at `addr` (see [`Bus::write_bytes`]).
    pub fn load(&mut self, addr: u32, bytes: &[u8]) {
        self.write_bytes(addr, bytes);
    }

    /// Side-effect free read of plain memory for debugger views; `None` for peripherals and
    /// unmapped addresses.
    pub fn peek(&self, addr: u32, size: u32) -> Option<u32> {
        let r = self.region(addr);
        if r.perm & PERM_R == 0 || r.kind != KIND_MEM {
            return None;
        }
        let d = &self.mems[r.target as usize].data;
        let i = (r.off + (addr - r.base)) as usize;
        let b = d.get(i..i + size as usize)?;
        Some(b.iter().enumerate().fold(0, |v, (k, &x)| v | (x as u32) << (8 * k)))
    }

    /// Reads one halfword for an instruction fetch (needs execute permission).
    fn fetch_half(&self, addr: u32) -> Result<u16, AccessFault> {
        let r = self.region(addr);
        if r.perm & PERM_X == 0 || r.kind != KIND_MEM {
            return Err(AccessFault);
        }
        let d = &self.mems[r.target as usize].data;
        let i = (r.off + (addr - r.base)) as usize;
        d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(AccessFault)
    }

    /// Index of the code page holding `addr`, allocating it on first use.
    pub(crate) fn code_page(&mut self, addr: u32) -> Result<u32, AccessFault> {
        let r = *self.region(addr);
        if r.perm & PERM_X == 0 || r.kind != KIND_MEM {
            return Err(AccessFault);
        }
        let m = &mut self.mems[r.target as usize];
        let page = ((r.off + (addr - r.base)) >> PAGE_SHIFT) as usize;
        let slot = m.code_idx.get_mut(page).ok_or(AccessFault)?;
        if *slot == NO_PAGE {
            *slot = (self.code.len() / SLOTS) as u32;
            self.code.resize(self.code.len() + SLOTS, Insn::UNDECODED);
            m.ncode += 1;
        }
        Ok(*slot)
    }

    /// Slow-path fetch: decodes the instruction at `addr` (2-byte aligned), caches it in the code
    /// page `page` (from [`Bus::code_page`]) and returns it. On a fault the address of the
    /// offending halfword is returned (the second half of a 32-bit instruction may fault).
    pub(crate) fn decode_at(&mut self, addr: u32, page: u32) -> Result<Insn, u32> {
        let lo = self.fetch_half(addr).map_err(|_| addr)? as u32;
        let word = if lo & 3 == 3 { lo | (self.fetch_half(addr.wrapping_add(2)).map_err(|_| addr.wrapping_add(2))? as u32) << 16 } else { lo };
        let insn = decode(word);
        debug_assert!(insn.op != Op::Undecoded);
        self.code[page as usize * SLOTS + (((addr >> 1) as usize) & (SLOTS - 1))] = insn;
        Ok(insn)
    }

    /// Discards all pre-decoded code (e.g. after replacing memory contents behind the bus's back).
    pub fn flush_code(&mut self) {
        self.code.fill(Insn::UNDECODED);
    }
}
