//! Machine-wide services shared by the ESP32-C3 peripherals: the GPIO electrical model (pins,
//! logic-analyzer trace), the clock tree as exact rational frequencies, the CPU interrupt controller
//! (the part of the interrupt matrix that drives the core's interrupt lines), SYSTEM register state,
//! warnings and the Serial Monitor output.
//!
//! The CPU cycle counter counts CPU clock cycles; [`Sys::clock`] converts cycles to seconds across
//! clock-tree changes. Peripherals schedule their events in CPU cycles and derive their tick lengths
//! from [`Clocks`] with exact integer ratios ([`Ticker`]), so a 16 MHz SYSTIMER tick is exactly 10
//! cycles of a 160 MHz CPU and 2.5 cycles of a 40 MHz one.

use std::collections::HashSet;

use crate::avr::Message;
use crate::pins::{ClockModel, Pin, PinTrace};

/// Number of GPIOs of the ESP32-C3 (GPIO0 - GPIO21).
pub const NGPIO: usize = 22;
/// Interrupt matrix peripheral sources (0 - 61).
pub const NSRC: usize = 62;

// ---- exact clock arithmetic ---------------------------------------------------------------------

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A frequency as the exact ratio `num / den` Hz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub num: u64,
    pub den: u64,
}

impl Rate {
    pub const fn hz(num: u64) -> Self {
        Self { num, den: 1 }
    }

    pub fn divided(self, d: u64) -> Self {
        Self { num: self.num, den: self.den * d }.reduced()
    }

    fn reduced(self) -> Self {
        let g = gcd(self.num, self.den).max(1);
        Self { num: self.num / g, den: self.den / g }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// CPU cycles per tick of a timer clocked by `src` through a divider of `div`: `(num, den)` with
    /// `cycles = ticks * num / den`.
    pub fn cycles_per_tick(self, src: Rate, div: u64) -> (u64, u64) {
        let n = self.num as u128 * src.den as u128 * div as u128;
        let d = self.den as u128 * src.num as u128;
        let g = gcd128(n, d).max(1);
        (((n / g) as u64).max(1), ((d / g) as u64).max(1))
    }
}

fn gcd128(a: u128, b: u128) -> u128 {
    if b == 0 {
        a
    } else {
        gcd128(b, a % b)
    }
}

pub const XTAL: Rate = Rate::hz(40_000_000);
pub const RC_FAST: Rate = Rate::hz(17_500_000);
/// SYSTIMER tick rate: XTAL_CLK / 2.5.
pub const SYSTIMER_HZ: Rate = Rate::hz(16_000_000);

/// Clock frequencies derived from the SYSTEM registers.
#[derive(Clone, Copy, Debug)]
pub struct Clocks {
    pub cpu: Rate,
    pub apb: Rate,
    pub xtal: Rate,
    pub rc_fast: Rate,
}

impl Clocks {
    /// Out of reset: XTAL_CLK / (PRE_DIV_CNT + 1) with PRE_DIV_CNT = 1 (SYSTEM_SYSCLK_CONF reset value).
    pub fn reset() -> Self {
        Self::from_regs(1, 0)
    }

    /// TRM "Reset and Clock", CPU clock: `SYSCLK_CONF` (`SOC_CLK_SEL` 0 XTAL_CLK / 1 PLL_CLK / 2 RC_FAST_CLK,
    /// `PRE_DIV_CNT`) and `CPU_PER_CONF.CPUPERIOD_SEL` (PLL: 0 = 80 MHz, 1 = 160 MHz). APB_CLK is 80 MHz under
    /// the PLL and equals CPU_CLK otherwise.
    pub fn from_regs(sysclk_conf: u32, cpu_per_conf: u32) -> Self {
        let div = (sysclk_conf & 0x3ff) as u64 + 1;
        let (cpu, apb) = match (sysclk_conf >> 10) & 3 {
            1 => (if cpu_per_conf & 3 == 0 { Rate::hz(80_000_000) } else { Rate::hz(160_000_000) }, Rate::hz(80_000_000)),
            2 => (RC_FAST.divided(div), RC_FAST.divided(div)),
            _ => (XTAL.divided(div), XTAL.divided(div)),
        };
        Self { cpu, apb, xtal: XTAL, rc_fast: RC_FAST }
    }
}

/// A free-running counter driven from the CPU cycle count through an exact rational ratio.
#[derive(Clone, Copy, Debug)]
pub struct Ticker {
    /// Cycle where `value` / `carry` were captured.
    base: u64,
    value: u64,
    /// Fraction of a tick already elapsed at `base`, in units of `1 / num` tick (scaled by `den`).
    carry: u128,
    /// CPU cycles per tick = `num / den`.
    num: u64,
    den: u64,
    pub running: bool,
    /// Count down instead of up.
    pub down: bool,
}

impl Ticker {
    pub fn new() -> Self {
        Self { base: 0, value: 0, carry: 0, num: 1, den: 1, running: false, down: false }
    }

    #[inline]
    fn advance(&self, cycle: u64) -> (u64, u128) {
        if !self.running {
            return (self.value, self.carry);
        }
        let e = cycle.saturating_sub(self.base) as u128 * self.den as u128 + self.carry;
        let ticks = (e / self.num as u128) as u64;
        let v = if self.down { self.value.wrapping_sub(ticks) } else { self.value.wrapping_add(ticks) };
        (v, e % self.num as u128)
    }

    /// Counter value at `cycle` (not masked: callers mask to their width).
    #[inline]
    pub fn value_at(&self, cycle: u64) -> u64 {
        self.advance(cycle).0
    }

    /// Captures the state at `cycle`, then applies a new ratio / run state.
    pub fn retime(&mut self, cycle: u64, ratio: (u64, u64), running: bool) {
        let (v, c) = self.advance(cycle);
        self.value = v;
        // The elapsed fraction keeps its meaning across a ratio change.
        self.carry = c * ratio.0 as u128 / self.num as u128;
        self.base = cycle;
        (self.num, self.den) = ratio;
        self.running = running;
    }

    /// Sets the counter to `value` at `cycle` (the fraction restarts).
    pub fn set(&mut self, cycle: u64, value: u64) {
        self.value = value;
        self.carry = 0;
        self.base = cycle;
    }

    /// First cycle (>= `now`) at which the counter has advanced to `target` or beyond, counting in the
    /// ticker's direction from its value at `now`. `None` when stopped or already past.
    pub fn cycle_of(&self, now: u64, target: u64, mask: u64) -> Option<u64> {
        if !self.running {
            return None;
        }
        let cur = self.value_at(now);
        let need = if self.down { cur.wrapping_sub(target) } else { target.wrapping_sub(cur) } & mask;
        let have = self.advance(now).1;
        let need_num = need as u128 * self.num as u128;
        if need_num <= have {
            return Some(now);
        }
        let cycles = (need_num - have).div_ceil(self.den as u128);
        Some(now + cycles as u64)
    }
}

impl Default for Ticker {
    fn default() -> Self {
        Self::new()
    }
}

// ---- CPU interrupt controller ---------------------------------------------------------------------

/// The ESP32-C3 interrupt matrix and CPU interrupt controller (INTERRUPT_CORE0): each peripheral source is
/// mapped to one of CPU interrupts 1 - 31, several sources may share one (OR). Each CPU interrupt has an
/// enable bit, a type (level / edge), a priority (0 - 15) and the controller a priority threshold. The
/// controller presents the single winning interrupt (highest priority, lowest number among equals) to the hart
/// as the matching `mip` bit.
pub struct Intc {
    /// CPU interrupt (0 = none) every source is mapped to.
    pub map: [u8; NSRC],
    /// Sources mapped to each CPU interrupt.
    line_srcs: [u64; 32],
    /// Current level of every source.
    src: u64,
    pub enable: u32,
    /// 1 = edge triggered.
    pub ty: u32,
    pub pri: [u8; 32],
    pub thresh: u8,
    /// Pending flag of every CPU interrupt (level: follows the sources; edge: latched until cleared).
    pending: u32,
    prev_level: u32,
    /// Mask last handed to the hart.
    sent: u32,
    /// Mask computed by the last update that the hart has not been told about.
    changed: Option<u32>,
}

impl Intc {
    pub fn new() -> Self {
        Self { map: [0; NSRC], line_srcs: [0; 32], src: 0, enable: 0, ty: 0, pri: [0; 32], thresh: 0, pending: 0, prev_level: 0, sent: 0, changed: None }
    }

    /// Power-on state; the sources currently asserted by the peripherals are kept by the caller (peripheral
    /// resets re-assert them).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn set_map(&mut self, src: usize, line: u8) {
        if src >= NSRC {
            return;
        }
        let line = line & 31;
        let old = self.map[src] as usize;
        self.line_srcs[old] &= !(1 << src);
        self.map[src] = line;
        self.line_srcs[line as usize] |= 1 << src;
        self.update();
    }

    /// Drives source `src`; returns true when the mask presented to the hart changed.
    pub fn set_source(&mut self, src: u8, level: bool) -> bool {
        let bit = 1u64 << (src as usize % 64);
        let was = self.src & bit != 0;
        if was == level {
            return false;
        }
        if level {
            self.src |= bit;
        } else {
            self.src &= !bit;
        }
        self.update();
        self.changed.is_some()
    }

    /// Status of the peripheral sources (`INTR_STATUS_REG_0/1`).
    pub fn source_status(&self) -> u64 {
        self.src
    }

    /// Pending CPU interrupts (`CPU_INT_EIP_STATUS`).
    pub fn pending(&self) -> u32 {
        self.pending
    }

    /// Clears the latched pending bits of edge-triggered interrupts (`CPU_INT_CLEAR`).
    pub fn clear(&mut self, mask: u32) {
        self.pending &= !(mask & self.ty);
        self.update();
    }

    /// Recomputes the pending flags and the winning interrupt.
    pub fn update(&mut self) {
        let mut level = 0u32;
        for n in 1..32usize {
            if self.line_srcs[n] & self.src != 0 {
                level |= 1 << n;
            }
        }
        let rising = level & !self.prev_level;
        self.prev_level = level;
        self.pending = (self.pending & self.ty) | (level & !self.ty) | (rising & self.ty);
        let mut best: Option<(u8, u32)> = None;
        let mut cand = self.pending & self.enable & !1;
        while cand != 0 {
            let n = cand.trailing_zeros();
            cand &= cand - 1;
            let p = self.pri[n as usize];
            if p < self.thresh {
                continue;
            }
            if best.is_none_or(|(bp, _)| p > bp) {
                best = Some((p, n));
            }
        }
        let mask = best.map_or(0, |(_, n)| 1u32 << n);
        if mask != self.sent {
            self.sent = mask;
            self.changed = Some(mask);
        }
    }

    pub fn take_changed(&mut self) -> Option<u32> {
        self.changed.take()
    }

    /// Mask currently presented to the hart.
    pub fn presented(&self) -> u32 {
        self.sent
    }
}

impl Default for Intc {
    fn default() -> Self {
        Self::new()
    }
}

// ---- machine-wide state ------------------------------------------------------------------------------

/// SYSTEM register values other peripherals depend on.
#[derive(Clone, Copy, Debug)]
pub struct SysRegs {
    pub clk_en0: u32,
    pub clk_en1: u32,
    pub rst0: u32,
    pub rst1: u32,
    pub sysclk_conf: u32,
    pub cpu_per_conf: u32,
}

impl SysRegs {
    pub const CLK_EN0_RESET: u32 = 0xf9c1_e06f;
    pub const CLK_EN1_RESET: u32 = 0x0000_0200;

    pub fn reset() -> Self {
        Self { clk_en0: Self::CLK_EN0_RESET, clk_en1: Self::CLK_EN1_RESET, rst0: 0, rst1: 0x0000_01fe, sysclk_conf: 1, cpu_per_conf: 0x0c }
    }
}

pub struct Sys {
    pub pins: Vec<Pin>,
    levels: Vec<u32>,
    pub trace: PinTrace,
    pub clock: ClockModel,
    pub clk: Clocks,
    pub vcc: f64,
    pub intc: Intc,
    pub regs: SysRegs,
    pub gpio: super::gpio::GpioState,
    /// Bytes decoded by the Serial Monitor (and written to the USB serial endpoint) since the last snapshot.
    pub serial_out: Vec<u8>,
    pub messages: Vec<Message>,
    warned: HashSet<String>,
    /// Pin level changes not yet delivered to the listening peripherals: (pin, level, cycle).
    pub changed: Vec<(u16, u8, u64)>,
    /// Peripherals reset through SYSTEM_PERIP_RST_ENx: (register 0 / 1, bit).
    pub resets: Vec<(u8, u8)>,
    /// Something needs the bus's attention after the current access (pin changes, clock changes, resets).
    pub attn: bool,
    pub clock_dirty: bool,
    /// Software requested a system reset (RTC_CNTL SW_SYS_RST).
    pub reset_req: bool,
}

impl Sys {
    pub fn new(gpio_count: usize) -> Self {
        let words = gpio_count.div_ceil(32).max(1);
        let clk = Clocks::reset();
        Self {
            pins: (0..gpio_count).map(|i| Pin::new(format!("GPIO{i}"))).collect(),
            levels: vec![0; words],
            trace: PinTrace::new(if gpio_count == 0 { 1 } else { 1 << 16 }, gpio_count),
            clock: ClockModel::new(clk.cpu.as_f64()),
            clk,
            vcc: 3.3,
            intc: Intc::new(),
            regs: SysRegs::reset(),
            gpio: super::gpio::GpioState::new(),
            serial_out: Vec::new(),
            messages: Vec::new(),
            warned: HashSet::new(),
            changed: Vec::new(),
            resets: Vec::new(),
            attn: false,
            clock_dirty: false,
            reset_req: false,
        }
    }

    // ---- messages ---------------------------------------------------------------------------------

    pub fn message(&mut self, cycle: u64, level: &'static str, text: impl Into<String>) {
        self.messages.push(Message { cycle, level, text: text.into() });
        if self.messages.len() > 200 {
            let excess = self.messages.len() - 200;
            self.messages.drain(..excess);
        }
    }

    /// De-duplicated warning (tight loops would otherwise flood the log).
    pub fn warn_key(&mut self, cycle: u64, key: impl Into<String>, text: impl Into<String>) {
        let key = key.into();
        if self.warned.contains(&key) {
            return;
        }
        if self.warned.len() > 1000 {
            self.warned.clear();
        }
        self.warned.insert(key);
        self.message(cycle, "warning", text);
    }

    // ---- clocks -------------------------------------------------------------------------------------

    /// Applies new clock-tree values at cycle `now`; peripherals are told through `on_clock_change` once the
    /// current bus access completes.
    pub fn set_clocks(&mut self, c: Clocks, now: u64) {
        let changed = self.clk.cpu != c.cpu || self.clk.apb != c.apb;
        self.clk = c;
        self.clock.set_hz(c.cpu.as_f64(), now);
        if changed {
            self.clock_dirty = true;
            self.attn = true;
        }
    }

    pub fn time_at(&self, cycle: u64) -> f64 {
        self.clock.time_at(cycle)
    }

    /// True when SYSTEM peripheral clock-enable bit `bit` of enable register `reg` (0 / 1) is set.
    #[inline]
    pub fn clock_on(&self, reg: u8, bit: u8) -> bool {
        let v = if reg == 0 { self.regs.clk_en0 } else { self.regs.clk_en1 };
        v >> bit & 1 != 0
    }

    // ---- pins ---------------------------------------------------------------------------------------

    /// Re-resolves the level of pin `i` after any change of its inputs; records the trace and queues a pin
    /// event when the level changed.
    pub fn update_pin(&mut self, i: usize, cycle: u64) {
        let vcc = self.vcc;
        let p = &mut self.pins[i];
        if !p.resolve(vcc) {
            return;
        }
        let lv = p.level;
        let (w, b) = (i >> 5, i & 31);
        self.levels[w] = (self.levels[w] & !(1 << b)) | (lv as u32) << b;
        self.trace.record(cycle, &self.levels);
        self.changed.push((i as u16, lv, cycle));
        self.attn = true;
    }

    /// Clears everything that belongs to the previous run (power cycle).
    pub fn power_on(&mut self) {
        self.trace.clear();
        self.levels.iter_mut().for_each(|w| *w = 0);
        for p in self.pins.iter_mut() {
            p.level = 0;
            p.volts = 0.0;
        }
        self.clk = Clocks::reset();
        self.clock.reset(self.clk.cpu.as_f64());
        self.regs = SysRegs::reset();
        self.intc.reset();
        self.changed.clear();
        self.resets.clear();
        self.messages.clear();
        self.warned.clear();
        self.serial_out.clear();
        self.attn = false;
        self.clock_dirty = false;
        self.reset_req = false;
    }

    /// Current logic levels of the pins as bit words (bit n = GPIO n).
    pub fn levels(&self) -> &[u32] {
        &self.levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_tree_from_the_system_registers() {
        let c = Clocks::reset();
        assert_eq!((c.cpu.as_f64(), c.apb.as_f64()), (20e6, 20e6), "XTAL / 2 out of reset");
        // PLL: CPUPERIOD_SEL 0 = 80 MHz, 1 = 160 MHz; APB 80 MHz either way.
        let c = Clocks::from_regs(1 << 10, 0);
        assert_eq!((c.cpu.as_f64(), c.apb.as_f64()), (80e6, 80e6));
        let c = Clocks::from_regs(1 << 10, 1);
        assert_eq!((c.cpu.as_f64(), c.apb.as_f64()), (160e6, 80e6));
        // XTAL with a divider of 3 is not an integer frequency but stays exact.
        let c = Clocks::from_regs(2, 0);
        assert_eq!((c.cpu.num, c.cpu.den), (40_000_000, 3));
        let c = Clocks::from_regs((2 << 10) | 3, 0);
        assert_eq!(c.cpu.as_f64(), 17.5e6 / 4.0);
    }

    #[test]
    fn cycles_per_tick_is_an_exact_ratio() {
        let cpu160 = Rate::hz(160_000_000);
        assert_eq!(cpu160.cycles_per_tick(SYSTIMER_HZ, 1), (10, 1));
        assert_eq!(XTAL.divided(2).cycles_per_tick(SYSTIMER_HZ, 1), (5, 4), "20 MHz CPU: 1.25 cycles per 16 MHz tick");
        assert_eq!(Rate::hz(80_000_000).cycles_per_tick(XTAL, 40), (80, 1), "TIMG: XTAL / 40 on an 80 MHz CPU");
    }

    #[test]
    fn ticker_counts_exactly_across_ratio_changes() {
        let mut t = Ticker::new();
        t.retime(0, (5, 4), true); // 1.25 cycles per tick
        assert_eq!((t.value_at(0), t.value_at(4), t.value_at(5), t.value_at(1000)), (0, 3, 4, 800));
        // First cycle at which the counter reaches 800: 1000.
        assert_eq!(t.cycle_of(0, 800, u64::MAX), Some(1000));
        assert_eq!(t.cycle_of(0, 801, u64::MAX), Some(1002));
        // Switch to 10 cycles per tick at cycle 1000 (value 800): the next tick comes 10 cycles later.
        t.retime(1000, (10, 1), true);
        assert_eq!((t.value_at(1000), t.value_at(1009), t.value_at(1010), t.value_at(2000)), (800, 800, 801, 900));
        // Stopping freezes the value; restarting continues from it.
        t.retime(1010, (10, 1), false);
        assert_eq!(t.value_at(5000), 801);
        t.retime(5000, (10, 1), true);
        assert_eq!(t.value_at(5100), 811);
        // Loading a value restarts the fraction.
        t.set(6000, 1_000_000);
        assert_eq!((t.value_at(6009), t.value_at(6010)), (1_000_000, 1_000_001));
    }

    #[test]
    fn ticker_counts_down_and_wraps() {
        let mut t = Ticker::new();
        t.down = true;
        t.retime(0, (1, 1), true);
        t.set(0, 3);
        assert_eq!(t.value_at(2), 1);
        assert_eq!(t.value_at(4) & 0xff, 0xff, "wraps below zero");
        assert_eq!(t.cycle_of(0, 0, 0xffff), Some(3));
    }

    #[test]
    fn interrupt_arbitration_by_priority_threshold_and_type() {
        let mut i = Intc::new();
        i.set_map(37, 10);
        i.set_map(21, 12);
        i.enable = 1 << 10 | 1 << 12;
        i.pri[10] = 3;
        i.pri[12] = 3;
        i.update();
        assert!(i.set_source(37, true));
        assert_eq!(i.take_changed(), Some(1 << 10));
        // Equal priorities: the lower CPU interrupt number wins; a higher priority beats it.
        assert!(!i.set_source(21, true));
        i.pri[12] = 9;
        i.update();
        assert_eq!(i.take_changed(), Some(1 << 12));
        // The threshold masks lower priorities.
        i.thresh = 10;
        i.update();
        assert_eq!(i.take_changed(), Some(0));
        i.thresh = 0;
        i.update();
        assert_eq!(i.take_changed(), Some(1 << 12));
        // Two sources on one line OR together.
        i.set_map(22, 12);
        i.set_source(21, false);
        assert_eq!(i.take_changed(), Some(1 << 10));
        i.set_source(22, true);
        assert_eq!(i.take_changed(), Some(1 << 12));
        // Edge type: latches the rising edge until cleared, even after the source drops.
        i.ty = 1 << 12;
        i.set_source(22, false);
        i.set_source(22, true);
        i.set_source(22, false);
        assert_eq!(i.pending() & (1 << 12), 1 << 12);
        i.clear(1 << 12);
        assert_eq!(i.pending() & (1 << 12), 0);
        assert_eq!(i.presented(), 1 << 10);
    }
}
