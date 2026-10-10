//! Nested Vectored Interrupt Controller: enable / pending / active state, priorities and priority
//! grouping for the system exceptions (1-15) and up to 240 external interrupts.
//!
//! References: ARM DDI 0403E.e B3.4 (NVIC), B1.5.4 (exception priorities) and the Cortex-M4
//! Devices Generic User Guide (ARM DUI 0553) section 4.3 for the register layout.
//!
//! Exception numbers: 1 Reset, 2 NMI, 3 HardFault, 4 MemManage, 5 BusFault, 6 UsageFault,
//! 11 SVCall, 12 DebugMon, 14 PendSV, 15 SysTick, 16+n IRQ n. Priorities are signed "group"
//! values: Reset/NMI/HardFault are -3/-2/-1, configurable ones are `prio >> (PRIGROUP + 1)`;
//! "no active exception" is [`IDLE_PRIO`]. A pending exception preempts when its group priority is
//! numerically lower than the execution priority.

pub const EXC_RESET: u16 = 1;
pub const EXC_NMI: u16 = 2;
pub const EXC_HARDFAULT: u16 = 3;
pub const EXC_MEMMANAGE: u16 = 4;
pub const EXC_BUSFAULT: u16 = 5;
pub const EXC_USAGEFAULT: u16 = 6;
pub const EXC_SVCALL: u16 = 11;
pub const EXC_DEBUGMON: u16 = 12;
pub const EXC_PENDSV: u16 = 14;
pub const EXC_SYSTICK: u16 = 15;

/// Execution priority when no exception is active and nothing is masked (above every priority).
pub const IDLE_PRIO: i16 = 256;

pub struct Nvic {
    /// Number of external interrupts.
    pub nirq: u32,
    /// Implemented priority bits (3..8); the low `8 - bits` bits of a priority read as zero.
    pub prio_bits: u8,
    pub enabled: [u32; 8],
    pub pending: [u32; 8],
    pub active: [u32; 8],
    /// Current level of the level-sensitive interrupt lines driven by peripherals.
    pub lines: [u32; 8],
    /// Pending system exceptions (bit = exception number).
    pub sys_pending: u32,
    /// Active system exceptions (bit = exception number).
    pub sys_active: u32,
    /// Raw priority bytes indexed by exception number (system handlers 4-15, IRQs from 16).
    pub prio: Vec<u8>,
    /// AIRCR.PRIGROUP.
    pub prigroup: u8,
    /// Something affecting the exception decision changed; the run loop re-evaluates when set.
    pub dirty: bool,
}

impl Nvic {
    pub fn new(nirq: u32, prio_bits: u8) -> Self {
        let nirq = nirq.min(240);
        Self {
            nirq,
            prio_bits: prio_bits.clamp(3, 8),
            enabled: [0; 8],
            pending: [0; 8],
            active: [0; 8],
            lines: [0; 8],
            sys_pending: 0,
            sys_active: 0,
            prio: vec![0; 16 + nirq as usize],
            prigroup: 0,
            dirty: false,
        }
    }

    pub fn reset(&mut self) {
        self.enabled = [0; 8];
        self.pending = [0; 8];
        self.active = [0; 8];
        self.lines = [0; 8];
        self.sys_pending = 0;
        self.sys_active = 0;
        self.prio.iter_mut().for_each(|p| *p = 0);
        self.prigroup = 0;
        self.dirty = false;
    }

    #[inline]
    fn prio_mask(&self) -> u8 {
        (0xffu16 << (8 - self.prio_bits)) as u8
    }

    /// Group priority of a raw 8-bit priority value.
    #[inline]
    pub fn group_of_raw(&self, raw: u8) -> i16 {
        ((raw as u16) >> (self.prigroup as u16 + 1).min(8)) as i16
    }

    fn sub_of_raw(&self, raw: u8) -> u16 {
        let sh = (self.prigroup as u16 + 1).min(8);
        (raw as u16) & ((1u16 << sh) - 1)
    }

    /// Group priority of exception `exc`.
    #[inline]
    pub fn group_prio(&self, exc: u16) -> i16 {
        match exc {
            1 => -3,
            2 => -2,
            3 => -1,
            _ => self.group_of_raw(self.prio.get(exc as usize).copied().unwrap_or(0)),
        }
    }

    // ---- line control -------------------------------------------------------------------

    pub fn set_pending(&mut self, irq: u32) {
        if irq < self.nirq {
            self.pending[(irq >> 5) as usize] |= 1 << (irq & 31);
            self.dirty = true;
        }
    }

    /// Drives interrupt line `irq`: a rising level latches the pending bit.
    pub fn set_line(&mut self, irq: u32, level: bool) {
        if irq >= self.nirq {
            return;
        }
        let (w, m) = ((irq >> 5) as usize, 1u32 << (irq & 31));
        let was = self.lines[w] & m != 0;
        if level {
            self.lines[w] |= m;
            if !was {
                self.set_pending(irq);
            }
        } else {
            self.lines[w] &= !m;
        }
    }

    /// Called when the handler of `irq` returns: a line that is still asserted pends again.
    pub fn resample_line(&mut self, irq: u32) {
        if irq < self.nirq && self.lines[(irq >> 5) as usize] >> (irq & 31) & 1 != 0 {
            self.set_pending(irq);
        }
    }

    pub fn clear_pending(&mut self, irq: u32) {
        if irq < self.nirq {
            self.pending[(irq >> 5) as usize] &= !(1 << (irq & 31));
            self.dirty = true;
        }
    }

    pub fn set_sys_pending(&mut self, exc: u16) {
        self.sys_pending |= 1 << exc;
        self.dirty = true;
    }

    pub fn clear_sys_pending(&mut self, exc: u16) {
        self.sys_pending &= !(1 << exc);
        self.dirty = true;
    }

    /// Marks exception `exc` active / inactive.
    pub fn set_active(&mut self, exc: u16, on: bool) {
        if exc < 16 {
            if on {
                self.sys_active |= 1 << exc;
            } else {
                self.sys_active &= !(1 << exc);
            }
        } else {
            let n = (exc - 16) as u32;
            let (w, b) = ((n >> 5) as usize, 1u32 << (n & 31));
            if on {
                self.active[w] |= b;
            } else {
                self.active[w] &= !b;
            }
        }
    }

    /// Clears the pending state of exception `exc` (taken).
    pub fn take_pending(&mut self, exc: u16) {
        if exc < 16 {
            self.sys_pending &= !(1 << exc);
        } else {
            let n = (exc - 16) as u32;
            self.pending[(n >> 5) as usize] &= !(1 << (n & 31));
        }
    }

    // ---- priority resolution -------------------------------------------------------------

    /// Highest-priority pending, enabled exception whose group priority is below `exec_prio`.
    /// Ties are broken by sub-priority, then by exception number.
    pub fn best_pending(&self, exec_prio: i16) -> Option<u16> {
        let mut best: Option<(i16, u16, u16)> = None; // (group, sub, exc)
        let mut consider = |exc: u16, this: &Nvic| {
            let (g, s) = match exc {
                1 => (-3, 0),
                2 => (-2, 0),
                3 => (-1, 0),
                _ => {
                    let raw = this.prio[exc as usize];
                    (this.group_of_raw(raw), this.sub_of_raw(raw))
                }
            };
            if g < exec_prio && best.is_none_or(|b| (g, s, exc) < b) {
                best = Some((g, s, exc));
            }
        };
        let mut sp = self.sys_pending & !self.sys_active;
        // A system exception that is both active and pending waits for its handler to finish.
        sp &= !1;
        while sp != 0 {
            let exc = sp.trailing_zeros() as u16;
            sp &= sp - 1;
            consider(exc, self);
        }
        for w in 0..((self.nirq as usize + 31) >> 5) {
            let mut bits = self.pending[w] & self.enabled[w] & !self.active[w];
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                consider(16 + (w as u16) * 32 + b as u16, self);
            }
        }
        best.map(|b| b.2)
    }

    /// Any enabled external interrupt or pending system exception exists (wake-up test).
    pub fn any_pending(&self) -> bool {
        if self.sys_pending & !1 != 0 {
            return true;
        }
        (0..((self.nirq as usize + 31) >> 5)).any(|w| self.pending[w] & self.enabled[w] != 0)
    }

    /// Highest-priority pending exception regardless of masking (ICSR.VECTPENDING).
    pub fn vect_pending(&self) -> u16 {
        self.best_pending(IDLE_PRIO + 1).unwrap_or(0)
    }

    // ---- registers (0xE000E100 - 0xE000E4EF, STIR at 0xE000EF00) ---------------------------

    /// Reads a NVIC register word at `off` = address - 0xE000_E000 (word aligned).
    pub fn read_word(&self, off: u32) -> u32 {
        let words = ((self.nirq + 31) >> 5) as usize;
        match off {
            0x100..=0x11f | 0x180..=0x19f => self.bitmap(&self.enabled, off, words),
            0x200..=0x21f | 0x280..=0x29f => self.bitmap(&self.pending, off, words),
            0x300..=0x31f => self.bitmap(&self.active, off, words),
            0x400..=0x4ef => {
                let n = (off - 0x400) as usize;
                let mut v = 0;
                for k in 0..4 {
                    if n + k < self.nirq as usize {
                        v |= (self.prio[16 + n + k] as u32) << (8 * k);
                    }
                }
                v
            }
            _ => 0,
        }
    }

    fn bitmap(&self, map: &[u32; 8], off: u32, words: usize) -> u32 {
        let w = ((off & 0x1f) >> 2) as usize;
        if w < words {
            map[w]
        } else {
            0
        }
    }

    /// Writes a NVIC register word.
    pub fn write_word(&mut self, off: u32, v: u32) {
        let words = ((self.nirq + 31) >> 5) as usize;
        let w = ((off & 0x1f) >> 2) as usize;
        match off {
            0x100..=0x11f if w < words => self.enabled[w] |= v,
            0x180..=0x19f if w < words => self.enabled[w] &= !v,
            0x200..=0x21f if w < words => self.pending[w] |= v,
            0x280..=0x29f if w < words => self.pending[w] &= !v,
            0x400..=0x4ef => {
                let n = (off - 0x400) as usize;
                let mask = self.prio_mask();
                for k in 0..4 {
                    if n + k < self.nirq as usize {
                        self.prio[16 + n + k] = (v >> (8 * k)) as u8 & mask;
                    }
                }
            }
            0xf00 => {
                if (v & 0x1ff) < self.nirq {
                    self.set_pending(v & 0x1ff);
                }
            }
            _ => return,
        }
        // Pending bits above `nirq` never exist.
        if self.nirq & 31 != 0 {
            let top = ((self.nirq + 31) >> 5) as usize - 1;
            let m = (1u32 << (self.nirq & 31)) - 1;
            self.enabled[top] &= m;
            self.pending[top] &= m;
        }
        self.dirty = true;
    }

    /// Byte write to the priority registers (IPR is byte accessible).
    pub fn write_prio_byte(&mut self, off: u32, v: u8) {
        let n = (off - 0x400) as usize;
        if n < self.nirq as usize {
            self.prio[16 + n] = v & self.prio_mask();
            self.dirty = true;
        }
    }

    /// Sets a system handler priority (SHPR1-3), `exc` in 4..=15.
    pub fn set_sys_prio(&mut self, exc: u16, v: u8) {
        if (4..16).contains(&exc) {
            self.prio[exc as usize] = v & self.prio_mask();
            self.dirty = true;
        }
    }
}
