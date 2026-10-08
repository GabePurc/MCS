//! Architecture-neutral GPIO electrical model, logic-analyzer trace and clock model.

use serde::{Deserialize, Serialize};

/// What the outside world connects to a pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExtDrive {
    /// Nothing connected (high impedance).
    #[default]
    Float,
    Low,
    High,
    /// Driven with an analog voltage (`ext_volts`).
    Analog,
}

/// Signal generator attached to a pin from outside: a square wave (`count == None`) or a burst of
/// `count` pulses. The pin idles low (high when `invert`) and each period starts with the active
/// level for `duty * period`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinGenerator {
    pub hz: f64,
    /// Active fraction of each period (0..1).
    pub duty: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default)]
    pub invert: bool,
}

impl PinGenerator {
    /// Lowest/highest accepted frequency (an edge every simulated cycle is the practical limit).
    pub const MIN_HZ: f64 = 0.01;
    pub const MAX_HZ: f64 = 20e6;

    /// Clamped copy (frequency range, duty strictly between 0 and 1).
    pub fn sanitized(mut self) -> Self {
        self.hz = if self.hz.is_finite() { self.hz.clamp(Self::MIN_HZ, Self::MAX_HZ) } else { 1000.0 };
        self.duty = if self.duty.is_finite() { self.duty.clamp(0.001, 0.999) } else { 0.5 };
        self
    }

    pub fn active_s(&self) -> f64 {
        self.duty / self.hz
    }

    pub fn idle_s(&self) -> f64 {
        (1.0 - self.duty) / self.hz
    }
}

#[derive(Clone, Debug, Default)]
pub struct Pin {
    pub name: String,
    // MCU side
    /// 1 = output.
    pub dir: u8,
    /// Output latch.
    pub out: u8,
    pub pullup: u8,
    /// A peripheral (e.g. timer OCx) overrides the output value.
    pub ov_enable: u8,
    pub ov_value: u8,
    /// Reserved by an alternate function that disables the GPIO driver (e.g. RESET, XTAL1).
    pub reserved: bool,
    /// Name of the reserving function ("RESET", "XTAL1"...).
    pub reserved_by: &'static str,
    /// A peripheral overrides the data direction (e.g. USART TXD/RXD, SPI MISO).
    pub ddoe: u8,
    /// Overriding direction (1 = output) while `ddoe` is set.
    pub ddov: u8,
    // Outside world
    pub ext: ExtDrive,
    pub ext_volts: f64,
    /// Signal generator driving `ext` (see the `stimulus` peripheral).
    pub gen: Option<PinGenerator>,
    // Resolved
    pub level: u8,
    pub volts: f64,
}

impl Pin {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }

    /// Value driven by the MCU when the pin is an output.
    pub fn driven(&self) -> u8 {
        if self.ov_enable != 0 { self.ov_value } else { self.out }
    }

    /// Direction after peripheral overrides (1 = output).
    #[inline]
    pub fn effective_dir(&self) -> u8 {
        if self.ddoe != 0 { self.ddov } else { self.dir }
    }

    /// Resolves logic level and voltage. Returns true when the level changed.
    /// Analog inputs use Schmitt-trigger thresholds (VIL = 0.3 Vcc, VIH = 0.6 Vcc) with hysteresis.
    pub fn resolve(&mut self, vcc: f64) -> bool {
        let prev = self.level;
        if self.effective_dir() != 0 && !self.reserved {
            let v = self.driven();
            self.level = v;
            self.volts = if v != 0 { vcc } else { 0.0 };
        } else {
            match self.ext {
                ExtDrive::Low => {
                    self.level = 0;
                    self.volts = 0.0;
                }
                ExtDrive::High => {
                    self.level = 1;
                    self.volts = vcc;
                }
                ExtDrive::Analog => {
                    let v = self.ext_volts.clamp(0.0, vcc);
                    self.volts = v;
                    if v >= 0.6 * vcc {
                        self.level = 1;
                    } else if v <= 0.3 * vcc {
                        self.level = 0;
                    }
                }
                ExtDrive::Float => {
                    if self.pullup != 0 {
                        self.level = 1;
                        self.volts = vcc;
                    } else {
                        // Floating input: keep the last level (real hardware is undefined).
                        self.volts = if self.level != 0 { vcc } else { 0.0 };
                    }
                }
            }
        }
        self.level != prev
    }

    /// The MCU drives the pin while something external drives the opposite level.
    pub fn contention(&self) -> bool {
        if self.effective_dir() == 0 || self.reserved {
            return false;
        }
        let v = self.driven();
        (self.ext == ExtDrive::Low && v == 1) || (self.ext == ExtDrive::High && v == 0)
    }
}

/// Ring buffer of pin-level changes: (cycle, bitmask of all pin levels).
pub struct PinTrace {
    cycles: Vec<u64>,
    levels: Vec<u32>,
    /// Total number of entries ever written (sequence of the next entry).
    pub seq: u64,
    last: Option<u32>,
}

impl PinTrace {
    pub fn new(capacity: usize) -> Self {
        Self { cycles: vec![0; capacity], levels: vec![0; capacity], seq: 0, last: None }
    }

    #[inline]
    pub fn record(&mut self, cycle: u64, levels: u32) {
        if self.last == Some(levels) {
            return;
        }
        self.last = Some(levels);
        let i = (self.seq % self.cycles.len() as u64) as usize;
        self.cycles[i] = cycle;
        self.levels[i] = levels;
        self.seq += 1;
    }

    pub fn clear(&mut self) {
        self.seq = 0;
        self.last = None;
    }

    /// Entries with sequence >= `since` (at most `max` of the newest ones).
    pub fn read_since(&self, since: u64, max: usize) -> (u64, Vec<u64>, Vec<u32>) {
        let cap = self.cycles.len() as u64;
        let from = since.max(self.seq.saturating_sub(cap)).max(self.seq.saturating_sub(max as u64));
        let n = (self.seq - from) as usize;
        let mut c = Vec::with_capacity(n);
        let mut l = Vec::with_capacity(n);
        for k in 0..n as u64 {
            let i = ((from + k) % cap) as usize;
            c.push(self.cycles[i]);
            l.push(self.levels[i]);
        }
        (from, c, l)
    }
}

/// Piecewise-constant clock: converts between CPU cycles and seconds across frequency changes.
#[derive(Clone, Debug)]
pub struct ClockModel {
    pub hz: f64,
    seg_cycle: u64,
    seg_time: f64,
}

impl ClockModel {
    pub fn new(hz: f64) -> Self {
        Self { hz, seg_cycle: 0, seg_time: 0.0 }
    }

    pub fn time_at(&self, cycle: u64) -> f64 {
        self.seg_time + (cycle as f64 - self.seg_cycle as f64) / self.hz
    }

    /// CPU cycle at which `time` (seconds) is reached at the current frequency.
    pub fn cycle_at(&self, time: f64) -> u64 {
        let c = self.seg_cycle as f64 + ((time - self.seg_time) * self.hz - 1e-9).ceil();
        if c < 0.0 { 0 } else { c as u64 }
    }

    /// Returns true when the frequency actually changed.
    pub fn set_hz(&mut self, hz: f64, now: u64) -> bool {
        if hz == self.hz {
            return false;
        }
        self.seg_time = self.time_at(now);
        self.seg_cycle = now;
        self.hz = hz;
        true
    }

    pub fn reset(&mut self, hz: f64) {
        *self = Self::new(hz);
    }
}
