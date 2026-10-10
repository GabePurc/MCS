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

use super::esp32c3::serial::SerialBridge;
use super::esp32c3::stimulus::Stimulus;
use super::esp32c3::sys::Sys;
use crate::scheduler::{EventKey, Scheduler};

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

/// Scheduler owner ids of the bus-level services (device owners are indices into the device table).
pub const BRIDGE_OWNER: u8 = 0xfe;
pub const STIM_OWNER: u8 = 0xfd;

/// Services handed to a peripheral during an access or an event.
pub struct Cx {
    /// CPU cycle counter at the time of the access / event.
    pub cycles: u64,
    /// Interrupt lines (bit n = line n) the peripheral asserts; applied to `mip` after the access.
    pub irq_raise: u32,
    /// Interrupt lines the peripheral deasserts.
    pub irq_lower: u32,
    /// Index of the calling device (event owner).
    pub owner: u8,
    /// Event scheduler shared by the devices (timed in CPU cycles).
    pub sched: Scheduler,
    /// Pins, clocks, interrupt matrix and the other machine-wide state of the ESP32-C3.
    pub sys: Sys,
    /// A device asked the run loop to stop (system reset).
    pub stop_req: bool,
}

impl Default for Cx {
    fn default() -> Self {
        Self { cycles: 0, irq_raise: 0, irq_lower: 0, owner: 0, sched: Scheduler::new(), sys: Sys::new(0), stop_req: false }
    }
}

impl Cx {
    /// Schedules (or re-schedules) this device's event `tag` at the absolute cycle `cycle`.
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

    /// Drives interrupt matrix source `src` to `level`; the resulting CPU interrupt line changes are
    /// queued in `irq_raise` / `irq_lower`.
    pub fn irq_source(&mut self, src: u8, level: bool) {
        if self.sys.intc.set_source(src, level) {
            self.push_irq();
        }
    }

    /// Re-evaluates the CPU interrupt controller after a register change.
    pub fn irq_update(&mut self) {
        self.sys.intc.update();
        self.push_irq();
    }

    fn push_irq(&mut self) {
        if let Some(mask) = self.sys.intc.take_changed() {
            self.irq_raise = mask;
            self.irq_lower = !mask & super::cpu::IRQ_MASK;
        }
    }

    /// Asks the run loop to stop after the current instruction (system reset).
    pub fn request_stop(&mut self) {
        self.stop_req = true;
        self.irq_lower |= 1;
    }
}

/// A memory-mapped peripheral. All timing is event driven: instead of being ticked, a device
/// schedules the cycle where something observable happens ([`Cx::schedule`]) and advances lazily when
/// software accesses its registers. A device that changes interrupt lines does so through [`Cx`]
/// ([`Cx::irq_source`]); plain test devices may set `irq_raise` / `irq_lower` directly.
pub trait Mmio: Send {
    /// Reads `size` (1, 2 or 4) bytes at `offset` from the start of the window.
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32;
    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx);
    /// A scheduled event (see [`Cx::schedule`]) is due.
    fn on_event(&mut self, _tag: u8, _cx: &mut Cx) {}
    /// System / power-on reset, or reset through the SYSTEM peripheral reset register.
    fn reset(&mut self, _cx: &mut Cx) {}
    /// The level of GPIO `pin` changed (only delivered to devices registered as listeners).
    fn on_pin(&mut self, _pin: usize, _level: u8, _cycle: u64, _cx: &mut Cx) {}
    /// The clock tree changed: re-derive timing from `cx.sys.clk`.
    fn on_clock_change(&mut self, _cx: &mut Cx) {}
    /// Side-effect free read of the 32-bit register at `offset` (debugger views).
    fn peek(&mut self, offset: u32, cx: &mut Cx) -> u32 {
        self.read(offset & !3, 4, cx)
    }
    /// Short status lines for the peripheral inspector.
    fn inspect(&self, _cx: &Cx) -> Vec<(String, String)> {
        Vec::new()
    }
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
    pub cx: Cx,
    /// Serial Monitor end of the UART cable and the pin signal generators (test bench, outside the MCU).
    pub bridge: SerialBridge,
    pub stim: Stimulus,
    /// Devices that receive [`Mmio::on_pin`].
    listeners: Vec<u8>,
    /// SYSTEM peripheral reset position (register 0/1, bit) of every device, parallel to `devs`.
    dev_reset: Vec<Option<(u8, u8)>>,
    pub dev_names: Vec<String>,
    scratch: Vec<(u16, u8, u64)>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    /// An empty address space (every access faults until memories are mapped).
    pub fn new() -> Self {
        Self {
            pages: vec![0; PAGES],
            regions: vec![NO_REGION],
            mems: Vec::new(),
            devs: Vec::new(),
            code: Vec::new(),
            cx: Cx::default(),
            bridge: SerialBridge::new(),
            stim: Stimulus::new(0),
            listeners: Vec::new(),
            dev_reset: Vec::new(),
            dev_names: Vec::new(),
            scratch: Vec::new(),
        }
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

    /// Number of devices.
    pub fn device_count(&self) -> usize {
        self.devs.len()
    }

    /// Maps a named peripheral. `reset` is its reset position in the SYSTEM peripheral reset registers,
    /// `listen` makes it receive pin changes.
    pub fn add_named(&mut self, name: &str, base: u32, size: u32, dev: Box<dyn Mmio>, reset: Option<(u8, u8)>, listen: bool) -> Result<usize, String> {
        let idx = self.add_device(base, size, dev)?;
        self.dev_names.push(name.to_string());
        self.dev_reset.push(reset);
        if listen {
            self.listeners.push(idx as u8);
        }
        Ok(idx)
    }

    /// Side-effect free read of the 32-bit peripheral register at `addr` (debugger views).
    pub fn peek_register(&mut self, addr: u32, cycles: u64) -> Option<u32> {
        let r = *self.region(addr);
        if r.perm & PERM_R == 0 || r.kind != KIND_DEV {
            return None;
        }
        self.cx.cycles = cycles;
        self.cx.owner = r.target as u8;
        Some(self.devs[r.target as usize].peek((addr - r.base) & !3, &mut self.cx))
    }

    /// Inspector lines of every device that has some, as `(name, lines)`.
    pub fn inspect_all(&mut self, cycles: u64) -> Vec<(String, Vec<(String, String)>)> {
        self.cx.cycles = cycles;
        let mut out = Vec::new();
        for (i, d) in self.devs.iter().enumerate() {
            let v = d.inspect(&self.cx);
            if !v.is_empty() {
                out.push((self.dev_names.get(i).cloned().unwrap_or_default(), v));
            }
        }
        out
    }

    /// Moves the start of a memory window onto its backing memory (`mem_off` into the memory,
    /// 4 KiB aligned): the simplified flash MMU. Returns false if no window starts at `base`.
    pub fn set_window_offset(&mut self, base: u32, mem_off: u32) -> bool {
        let idx = self.pages[(base >> PAGE_SHIFT) as usize] as usize;
        if idx == 0 || self.regions[idx].base != base || !mem_off.is_multiple_of(PAGE_SIZE) {
            return false;
        }
        self.regions[idx].off = mem_off;
        self.flush_code();
        true
    }

    /// The whole contents of a backing memory.
    pub fn mem_data(&self, mem: MemId) -> &[u8] {
        &self.mems[mem.0].data
    }

    pub fn mem_data_mut(&mut self, mem: MemId) -> &mut [u8] {
        &mut self.mems[mem.0].data
    }

    // ---- scheduled events and machine-wide side effects ----------------------------------------

    /// Dispatches every scheduler event due at or before `now`.
    pub fn service(&mut self, now: u64) {
        while let Some((key, at)) = self.cx.sched.pop_due(now) {
            self.cx.cycles = at;
            self.cx.owner = key.owner;
            match key.owner {
                BRIDGE_OWNER => self.bridge.on_event(key.tag, &mut self.cx),
                STIM_OWNER => self.stim.on_event(key.tag, &mut self.cx),
                o => {
                    if let Some(d) = self.devs.get_mut(o as usize) {
                        d.on_event(key.tag, &mut self.cx);
                    }
                }
            }
            if self.cx.sys.attn {
                self.after_io(now);
            }
        }
        self.cx.cycles = now;
    }

    /// Work that follows a register access or event: peripheral resets requested through SYSTEM, clock-tree
    /// changes and pin level changes are delivered to the interested devices.
    #[inline(never)]
    pub fn after_io(&mut self, now: u64) {
        self.cx.sys.attn = false;
        while let Some((reg, bit)) = self.cx.sys.resets.pop() {
            for d in 0..self.devs.len() {
                if self.dev_reset[d] == Some((reg, bit)) {
                    self.cx.owner = d as u8;
                    self.cx.cycles = now;
                    self.devs[d].reset(&mut self.cx);
                }
            }
        }
        let mut rounds = 0;
        while self.cx.sys.clock_dirty && rounds < 4 {
            self.cx.sys.clock_dirty = false;
            rounds += 1;
            self.cx.cycles = now;
            for d in 0..self.devs.len() {
                self.cx.owner = d as u8;
                self.devs[d].on_clock_change(&mut self.cx);
            }
            self.cx.owner = BRIDGE_OWNER;
            self.bridge.on_clock_change(&mut self.cx);
            self.cx.owner = STIM_OWNER;
            self.stim.on_clock_change(&mut self.cx);
        }
        self.cx.sys.clock_dirty = false;
        let mut rounds = 0;
        while !self.cx.sys.changed.is_empty() && rounds < 64 {
            rounds += 1;
            let mut ev = std::mem::take(&mut self.scratch);
            std::mem::swap(&mut ev, &mut self.cx.sys.changed);
            for &(pin, level, cycle) in &ev {
                let at = now.max(cycle);
                self.cx.cycles = at;
                self.cx.sys.gpio_pin_changed(pin as usize, level, at);
                self.cx.gpio_irq_sync();
                for k in 0..self.listeners.len() {
                    let d = self.listeners[k] as usize;
                    self.cx.owner = d as u8;
                    self.devs[d].on_pin(pin as usize, level, cycle, &mut self.cx);
                }
                self.cx.owner = BRIDGE_OWNER;
                self.bridge.on_pin(pin as usize, level, cycle, &mut self.cx);
            }
            ev.clear();
            self.scratch = ev;
        }
        self.cx.sys.changed.clear();
        self.cx.sys.attn = false;
        self.cx.cycles = now;
    }

    /// Resets all devices (power-on or system reset; pin generators keep running across the latter).
    pub fn reset_devices(&mut self, now: u64, power_on: bool) {
        self.cx.cycles = now;
        for d in 0..self.devs.len() {
            self.cx.owner = d as u8;
            self.devs[d].reset(&mut self.cx);
        }
        self.cx.owner = BRIDGE_OWNER;
        self.bridge.reset(&mut self.cx);
        self.cx.owner = STIM_OWNER;
        self.stim.reset(power_on, &mut self.cx);
        self.after_io(now);
    }

    /// The ESP32-C3 memory map (TRM "System and Memory", internal memory): boot ROM (384 KiB IBUS, 128 KiB
    /// DBUS; both empty), flash cache windows (IROM / DROM, `flash_size` bytes), 400 KiB SRAM seen as
    /// IRAM and DRAM (SRAM0 16 KiB IRAM only + SRAM1 384 KiB on both buses) and the 8 KiB RTC fast memory.
    /// The ROM windows are mapped without execute permission: the machine stops with
    /// [`StopReason::RomCall`](super::StopReason::RomCall) when the pc enters them. Peripherals
    /// (0x6000_0000 and up) are added by the caller. Returns the bus plus the backing memories
    /// `(flash, sram)`.
    pub fn esp32c3(flash_size: u32) -> (Bus, MemId, MemId) {
        let mut bus = Bus::new();
        let flash = bus.add_mem(flash_size.clamp(PAGE_SIZE, 8 << 20));
        let sram = bus.add_mem(0x64000);
        let rom0 = bus.add_mem(0x60000);
        let rom1 = bus.add_mem(0x20000);
        let rtc = bus.add_mem(0x2000);
        let fs = bus.mem_size(flash);
        let ok = "esp32c3 memory map is static";
        bus.map(0x4200_0000, fs, flash, 0, PERM_RX).expect(ok);
        bus.map(0x3c00_0000, fs, flash, 0, PERM_R).expect(ok);
        bus.map(0x4037_c000, 0x64000, sram, 0, PERM_RWX).expect(ok);
        bus.map(0x3fc8_0000, 0x60000, sram, 0x4000, PERM_RW).expect(ok);
        bus.map(0x4000_0000, 0x60000, rom0, 0, PERM_R).expect(ok);
        bus.map(0x3ff0_0000, 0x20000, rom1, 0, PERM_R).expect(ok);
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
        if self.cx.sched.next <= cycles {
            self.service(cycles);
        }
        self.cx.cycles = cycles;
        self.cx.owner = dev as u8;
        let v = self.devs[dev as usize].read(off, size as u8, &mut self.cx) & size_mask(size);
        if self.cx.sys.attn {
            self.after_io(cycles);
        }
        v
    }

    #[inline(never)]
    fn dev_write(&mut self, dev: u32, off: u32, size: u32, value: u32, cycles: u64) {
        if self.cx.sched.next <= cycles {
            self.service(cycles);
        }
        self.cx.cycles = cycles;
        self.cx.owner = dev as u8;
        self.devs[dev as usize].write(off, size as u8, value & size_mask(size), &mut self.cx);
        if self.cx.sys.attn {
            self.after_io(cycles);
        }
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
