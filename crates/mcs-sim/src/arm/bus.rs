//! ARMv7-M memory map: code (flash), SRAM, memory-mapped peripherals and the private peripheral
//! bus (System Control Space).
//!
//! Regions are selected by the top address byte (`top` table), so a RAM or flash access costs one
//! table lookup plus a bounds check; peripherals are found by binary search over their (sorted)
//! address ranges with a one-entry cache. Region sizes and bases come from [`MemConfig`] so the
//! STM32 device descriptions can configure them. Regions must lie in distinct 16 MiB windows
//! (`addr >> 24`), which holds for STM32 (flash 0x0800_0000, SRAM 0x2000_0000, CCM/SRAM2 windows).
//!
//! Reference: ARM DDI 0403E.e B3.1 (system address map).

use crate::scheduler::{EventKey, Scheduler};

use super::nvic::Nvic;

/// Memory sizes and bases of one device.
#[derive(Clone, Debug)]
pub struct MemConfig {
    pub flash_base: u32,
    pub flash_size: u32,
    /// Flash is also visible at address 0 (boot alias).
    pub flash_alias: bool,
    /// RAM regions `(base, size)`.
    pub ram: Vec<(u32, u32)>,
}

impl Default for MemConfig {
    /// STM32G4-like defaults: 128 KiB flash at 0x0800_0000 (aliased at 0), 32 KiB SRAM.
    fn default() -> Self {
        Self { flash_base: 0x0800_0000, flash_size: 128 * 1024, flash_alias: true, ram: vec![(0x2000_0000, 32 * 1024)] }
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
}

impl Cx<'_> {
    /// Schedules (or re-schedules) this peripheral's event `tag` at the absolute cycle `cycle`.
    pub fn schedule(&mut self, tag: u8, cycle: u64) {
        self.sched.at(EventKey { owner: self.owner, tag }, cycle);
    }

    pub fn cancel(&mut self, tag: u8) {
        self.sched.cancel(EventKey { owner: self.owner, tag });
    }

    /// Latches IRQ line `irq` (0-based external interrupt number) as pending.
    pub fn raise_irq(&mut self, irq: u32) {
        self.nvic.set_pending(irq);
    }

    /// Clears the pending state of IRQ line `irq`.
    pub fn clear_irq(&mut self, irq: u32) {
        self.nvic.clear_pending(irq);
    }
}

/// A memory-mapped peripheral. All timing is event driven: instead of being ticked, a peripheral
/// schedules the cycle where something observable happens (`Cx::schedule`) and advances lazily
/// when software accesses its registers.
pub trait Mmio {
    /// Reads `size` (1, 2 or 4) bytes at `offset` from the peripheral base.
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32;
    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx);
    /// A scheduled event (see `Cx::schedule`) is due.
    fn on_event(&mut self, _tag: u8, _cx: &mut Cx) {}
    fn reset(&mut self) {}
}

pub(crate) const T_NONE: u8 = 0;
pub(crate) const T_FLASH: u8 = 1;
pub(crate) const T_ALIAS: u8 = 2;
pub(crate) const T_PERIPH: u8 = 3;
pub(crate) const T_PPB: u8 = 4;
/// RAM regions are `T_RAM + index`.
pub(crate) const T_RAM: u8 = 8;

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
    pub(crate) top: [u8; 256],
    ranges: Vec<PeriphEntry>,
    pub(crate) devs: Vec<Box<dyn Mmio>>,
    last: usize,
}

impl Bus {
    pub fn new(cfg: &MemConfig) -> Self {
        let mut top = [T_NONE; 256];
        top[0x40..0x60].fill(T_PERIPH);
        top[0xe0] = T_PPB;
        let mut ram = Vec::new();
        for (k, &(base, size)) in cfg.ram.iter().enumerate() {
            top[(base >> 24) as usize] = T_RAM + k as u8;
            ram.push(Ram { base, data: vec![0; size as usize] });
        }
        top[(cfg.flash_base >> 24) as usize] = T_FLASH;
        if cfg.flash_alias && cfg.flash_base >> 24 != 0 {
            top[0] = T_ALIAS;
        }
        Self {
            flash: vec![0xff; cfg.flash_size as usize],
            flash_base: cfg.flash_base,
            flash_size: cfg.flash_size,
            flash_alias: cfg.flash_alias,
            ram,
            top,
            ranges: Vec::new(),
            devs: Vec::new(),
            last: 0,
        }
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
        let t = self.top[(addr >> 24) as usize];
        let (data, off): (&[u8], u32) = match t {
            T_FLASH => (&self.flash, addr.wrapping_sub(self.flash_base)),
            T_ALIAS => (&self.flash, addr),
            t if t >= T_RAM => {
                let r = &self.ram[(t - T_RAM) as usize];
                (&r.data, addr.wrapping_sub(r.base))
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
        let t = self.top[(addr >> 24) as usize];
        if t < T_RAM {
            return false;
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
        let t = self.top[(addr >> 24) as usize];
        if t < T_RAM {
            return None;
        }
        let r = &self.ram[(t - T_RAM) as usize];
        let o = addr.wrapping_sub(r.base) as usize;
        r.data.get(o..o + len)
    }
}
