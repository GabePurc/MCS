//! ARMv7-M memory map: code (flash), SRAM, memory-mapped peripherals and the private peripheral
//! bus (System Control Space).
//!
//! Regions are selected by a page table indexed by `addr >> 20` (4096 one-byte entries, 4 KiB), so a
//! RAM or flash access costs one table lookup plus a bounds check no matter how many regions a device
//! has; peripherals are found by binary search over their (sorted) address ranges with a one-entry
//! cache. Region sizes and bases come from [`MemConfig`] so the STM32 device descriptions can
//! configure them. Every region owns whole 1 MiB pages (no two regions may share one; `Bus::new`
//! panics otherwise), which holds for STM32 (G4: flash 0x0800_0000, SRAM 0x2000_0000, CCM 0x1000_0000;
//! H7: ITCM 0, flash 0x0800_0000, DTCM 0x2000_0000, AXI SRAM 0x2400_0000, SRAM1-3 0x3000_0000 and its
//! 0x1000_0000 alias, SRAM4 0x3800_0000, backup SRAM 0x3880_0000).
//!
//! Reference: ARM DDI 0403E.e B3.1 (system address map).

use crate::scheduler::{EventKey, Scheduler};

use super::nvic::Nvic;
use super::sys::ArmSys;

/// Memory sizes and bases of one device.
#[derive(Clone, Debug)]
pub struct MemConfig {
    pub flash_base: u32,
    pub flash_size: u32,
    /// Flash is also visible at address 0 (boot alias).
    pub flash_alias: bool,
    /// RAM regions `(base, size)`.
    pub ram: Vec<(u32, u32)>,
    /// Parts of RAM regions that are also visible in a window of their own (STM32G4 CCM SRAM, STM32H7
    /// SRAM1-3 at 0x1000_0000). At most 8.
    pub ram_alias: Vec<RamAlias>,
}

/// A second address window onto `size` bytes of RAM region `ram`, starting at offset `off`.
#[derive(Clone, Copy, Debug)]
pub struct RamAlias {
    pub base: u32,
    pub size: u32,
    pub ram: usize,
    pub off: u32,
}

impl Default for MemConfig {
    /// STM32G4-like defaults: 128 KiB flash at 0x0800_0000 (aliased at 0), 32 KiB SRAM.
    fn default() -> Self {
        Self { flash_base: 0x0800_0000, flash_size: 128 * 1024, flash_alias: true, ram: vec![(0x2000_0000, 32 * 1024)], ram_alias: Vec::new() }
    }
}

/// Services handed to a memory-mapped peripheral during an access or an event.
pub struct Cx<'a> {
    /// CPU cycle counter at the time of the access / event.
    pub cycles: u64,
    pub nvic: &'a mut Nvic,
    pub sched: &'a mut Scheduler,
    /// Index of the calling peripheral (event owner).
    pub owner: u8,
    /// Pins, clock tree, clock gating and other machine-wide services.
    pub sys: &'a mut ArmSys,
}

impl Cx<'_> {
    /// Schedules (or re-schedules) this peripheral's event `tag` at the absolute cycle `cycle`.
    pub fn schedule(&mut self, tag: u8, cycle: u64) {
        self.sched.at(EventKey { owner: self.owner, tag }, cycle);
    }

    pub fn cancel(&mut self, tag: u8) {
        self.sched.cancel(EventKey { owner: self.owner, tag });
    }

    /// Cycle of the access / event.
    #[inline]
    pub fn now(&self) -> u64 {
        self.cycles
    }

    /// Simulated time (s) of the access / event.
    pub fn time_seconds(&self) -> f64 {
        self.sys.clock.time_at(self.cycles)
    }

    /// Latches IRQ line `irq` (0-based external interrupt number) as pending.
    pub fn raise_irq(&mut self, irq: u32) {
        self.nvic.set_pending(irq);
    }

    /// Clears the pending state of IRQ line `irq`.
    pub fn clear_irq(&mut self, irq: u32) {
        self.nvic.clear_pending(irq);
    }

    /// Drives the level of IRQ line `irq`. A rising level pends the interrupt; if the level is still
    /// high when its handler returns, the interrupt is pended again (level-sensitive sources).
    pub fn set_irq_line(&mut self, irq: u32, level: bool) {
        self.nvic.set_line(irq, level);
    }
}

/// A memory-mapped peripheral. All timing is event driven: instead of being ticked, a peripheral
/// schedules the cycle where something observable happens (`Cx::schedule`) and advances lazily
/// when software accesses its registers.
pub trait Mmio: Send {
    /// Reads `size` (1, 2 or 4) bytes at `offset` from the peripheral base.
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32;
    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx);
    /// A scheduled event (see `Cx::schedule`) is due.
    fn on_event(&mut self, _tag: u8, _cx: &mut Cx) {}
    /// System / power-on reset.
    fn reset(&mut self, _cx: &mut Cx) {}
    /// The level of GPIO `pin` changed (only delivered to peripherals registered as listeners).
    fn on_pin(&mut self, _pin: usize, _level: u8, _cycle: u64, _cx: &mut Cx) {}
    /// RCC changed the clock tree (HCLK or an APB prescaler): re-derive timing from `cx.sys.clk`.
    fn on_clock_change(&mut self, _cx: &mut Cx) {}
    /// Side-effect free read of the 32-bit register at `offset` (debugger views). Registers
    /// whose normal read has side effects override this.
    fn peek(&mut self, offset: u32, cx: &mut Cx) -> u32 {
        self.read(offset & !3, 4, cx)
    }
    /// Short status lines for the peripheral inspector.
    fn inspect(&self, _cx: &Cx) -> Vec<(String, String)> {
        Vec::new()
    }
}

pub(crate) const T_NONE: u8 = 0;
pub(crate) const T_FLASH: u8 = 1;
pub(crate) const T_ALIAS: u8 = 2;
pub(crate) const T_PERIPH: u8 = 3;
pub(crate) const T_PPB: u8 = 4;
/// RAM aliases are `T_RALIAS + index` (index < 8).
pub(crate) const T_RALIAS: u8 = 8;
/// RAM regions are `T_RAM + index`.
pub(crate) const T_RAM: u8 = 16;
/// log2 of the page size of the region table.
const PAGE_SHIFT: u32 = 20;
const PAGES: usize = 1 << (32 - PAGE_SHIFT);

pub struct Ram {
    pub base: u32,
    pub data: Vec<u8>,
}

struct PeriphEntry {
    start: u32,
    end: u32,
    dev: usize,
}

pub struct Bus {
    pub flash: Vec<u8>,
    pub flash_base: u32,
    pub flash_size: u32,
    pub flash_alias: bool,
    pub ram: Vec<Ram>,
    pub ram_alias: Vec<RamAlias>,
    /// Region code of every 1 MiB page of the address space.
    top: [u8; PAGES],
    ranges: Vec<PeriphEntry>,
    pub(crate) devs: Vec<Box<dyn Mmio>>,
    last: usize,
}

impl Bus {
    pub fn new(cfg: &MemConfig) -> Self {
        assert!(cfg.ram.len() < (256 - T_RAM as usize) && cfg.ram_alias.len() <= (T_RAM - T_RALIAS) as usize, "too many memory regions");
        let mut top = [T_NONE; PAGES];
        // Peripheral window 0x4000_0000-0x5FFF_FFFF and the whole private peripheral bus window.
        top[0x400..0x600].fill(T_PERIPH);
        top[0xe00..0xf00].fill(T_PPB);
        let mut claim = |base: u32, size: u32, code: u8| {
            let (first, last) = ((base >> PAGE_SHIFT) as usize, (base.wrapping_add(size.max(1) - 1) >> PAGE_SHIFT) as usize);
            for (page, entry) in top.iter_mut().enumerate().take(last + 1).skip(first) {
                assert_eq!(*entry, T_NONE, "memory regions overlap in the 1 MiB page at {:#010x}", (page as u32) << PAGE_SHIFT);
                *entry = code;
            }
        };
        let mut ram = Vec::new();
        for (k, &(base, size)) in cfg.ram.iter().enumerate() {
            claim(base, size, T_RAM + k as u8);
            ram.push(Ram { base, data: vec![0; size as usize] });
        }
        for (k, a) in cfg.ram_alias.iter().enumerate() {
            claim(a.base, a.size, T_RALIAS + k as u8);
        }
        claim(cfg.flash_base, cfg.flash_size, T_FLASH);
        if cfg.flash_alias && cfg.flash_base >> PAGE_SHIFT != 0 {
            claim(0, cfg.flash_size, T_ALIAS);
        }
        Self {
            flash: vec![0xff; cfg.flash_size as usize],
            flash_base: cfg.flash_base,
            flash_size: cfg.flash_size,
            flash_alias: cfg.flash_alias,
            ram,
            ram_alias: cfg.ram_alias.clone(),
            top,
            ranges: Vec::new(),
            devs: Vec::new(),
            last: 0,
        }
    }

    /// Region code (`T_*`) of the page containing `addr`.
    #[inline]
    pub(crate) fn kind(&self, addr: u32) -> u8 {
        self.top[(addr >> PAGE_SHIFT) as usize]
    }

    /// Maps `dev` over `[base, base + size)`; returns the device (event owner) index.
    pub fn add_peripheral(&mut self, base: u32, size: u32, dev: Box<dyn Mmio>) -> u8 {
        let idx = self.devs.len();
        self.devs.push(dev);
        let pos = self.ranges.partition_point(|r| r.start < base);
        self.ranges.insert(pos, PeriphEntry { start: base, end: base.saturating_add(size), dev: idx });
        self.last = 0;
        idx as u8
    }

    /// Finds the peripheral covering `addr`: (device index, offset).
    #[inline]
    pub(crate) fn find(&mut self, addr: u32) -> Option<(usize, u32)> {
        if let Some(r) = self.ranges.get(self.last) {
            if addr >= r.start && addr < r.end {
                return Some((r.dev, addr - r.start));
            }
        }
        let p = self.ranges.partition_point(|r| r.start <= addr);
        if p == 0 {
            return None;
        }
        let r = &self.ranges[p - 1];
        if addr < r.end {
            self.last = p - 1;
            Some((r.dev, addr - r.start))
        } else {
            None
        }
    }

    /// Reads plain memory (flash / RAM). `None` for peripheral, PPB or unmapped addresses.
    #[inline]
    pub fn read_mem(&self, addr: u32, size: u32) -> Option<u32> {
        let t = self.kind(addr);
        let (data, off): (&[u8], u32) = match t {
            T_FLASH => (&self.flash, addr.wrapping_sub(self.flash_base)),
            T_ALIAS => (&self.flash, addr),
            t if t >= T_RAM => {
                let r = &self.ram[(t - T_RAM) as usize];
                (&r.data, addr.wrapping_sub(r.base))
            }
            t if t >= T_RALIAS => {
                let a = self.ram_alias.get((t - T_RALIAS) as usize)?;
                let d = &self.ram.get(a.ram)?.data;
                (d.get(a.off as usize..(a.off + a.size) as usize)?, addr.wrapping_sub(a.base))
            }
            _ => return None,
        };
        let o = off as usize;
        match size {
            4 => data.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            2 => data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32),
            _ => data.get(o).map(|&b| b as u32),
        }
    }

    /// Writes RAM. Returns false for flash, peripherals and unmapped addresses.
    #[inline]
    pub fn write_ram(&mut self, addr: u32, size: u32, value: u32) -> bool {
        let t = self.kind(addr);
        if t < T_RAM {
            return t >= T_RALIAS && self.write_alias(t - T_RALIAS, addr, size, value);
        }
        let r = &mut self.ram[(t - T_RAM) as usize];
        let o = addr.wrapping_sub(r.base) as usize;
        match size {
            4 => match r.data.get_mut(o..o + 4) {
                Some(b) => {
                    b.copy_from_slice(&value.to_le_bytes());
                    true
                }
                None => false,
            },
            2 => match r.data.get_mut(o..o + 2) {
                Some(b) => {
                    b.copy_from_slice(&(value as u16).to_le_bytes());
                    true
                }
                None => false,
            },
            _ => match r.data.get_mut(o) {
                Some(b) => {
                    *b = value as u8;
                    true
                }
                None => false,
            },
        }
    }

    #[cold]
    fn write_alias(&mut self, idx: u8, addr: u32, size: u32, value: u32) -> bool {
        let Some(&a) = self.ram_alias.get(idx as usize) else { return false };
        let o = addr.wrapping_sub(a.base);
        if o.saturating_add(size) > a.size {
            return false;
        }
        let Some(r) = self.ram.get_mut(a.ram) else { return false };
        let i = (a.off + o) as usize;
        let b = value.to_le_bytes();
        match r.data.get_mut(i..i + size as usize) {
            Some(d) => {
                d.copy_from_slice(&b[..size as usize]);
                true
            }
            None => false,
        }
    }

    /// Offset into `flash` for a code address (primary window or boot alias).
    #[inline]
    pub fn flash_offset(&self, addr: u32) -> Option<u32> {
        let o = addr.wrapping_sub(self.flash_base);
        if o < self.flash_size {
            return Some(o);
        }
        if self.flash_alias && addr < self.flash_size {
            return Some(addr);
        }
        None
    }

    /// Byte slice of RAM starting at `addr` (for instruction fetch from RAM).
    pub fn ram_slice(&self, addr: u32, len: usize) -> Option<&[u8]> {
        let t = self.kind(addr);
        if t < T_RAM {
            if t >= T_RALIAS {
                let a = self.ram_alias.get((t - T_RALIAS) as usize)?;
                let o = (addr.wrapping_sub(a.base) + a.off) as usize;
                return self.ram.get(a.ram)?.data.get(o..o + len);
            }
            return None;
        }
        let r = &self.ram[(t - T_RAM) as usize];
        let o = addr.wrapping_sub(r.base) as usize;
        r.data.get(o..o + len)
    }
}
