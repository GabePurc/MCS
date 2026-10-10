//! System control of the classic AVRs (ATmega48/88/168/328 family, ATtiny25/45/85): clock source
//! selected by the CKSEL fuses, the CLKPR prescaler (CLKPCE timed sequence, CKDIV8 reset value),
//! reset flags (MCUSR), brown-out detection (BODLEVEL fuses), MCUCR (pull-up disable, interrupt
//! vector select with the IVCE timed sequence, BOD sleep), power reduction and the ATtiny85 PLL.
//!
//! Sources: DS40002061B sections 9 (clock), 11 (power), 12 (reset, BOD), 12.9 (MCUSR),
//! 13.1 (IVSEL); Atmel-2586Q sections 6 (clock, PLL), 7 (power), 8 (reset); Atmel-8126F
//! (ATtiny13A), Atmel-8183F (ATtiny24A/44A/84A) and Atmel-8246B (ATtiny2313A/4313) clock and
//! power sections (same structure, other CKSEL tables and clock frequencies).

use crate::avr::machine::{Cx, Event, Peripheral, ResetSource};

/// Clock source chosen by CKSEL3:0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockSource {
    /// Calibrated internal RC oscillator (8 MHz; 9.6 MHz on the ATtiny13A).
    Rc8M,
    /// Internal 128 kHz oscillator.
    Rc128k,
    /// External clock on XTAL1/CLKI (frequency set in Supply & Clock).
    External,
    /// Crystal / ceramic resonator on XTAL1/XTAL2 (frequency set in Supply & Clock).
    Crystal,
    /// 32.768 kHz watch crystal.
    LowFreqCrystal,
    /// ATtiny85 high-frequency PLL clock (64 MHz / 4).
    Pll16M,
    /// ATtiny15 compatibility mode (PLL 6.4 MHz).
    Rc6M4,
    /// Internal RC at half the nominal frequency (ATtiny13A 4.8 MHz, ATtiny2313A/4313 4 MHz).
    RcHalf,
}

impl ClockSource {
    fn label(self) -> &'static str {
        match self {
            Self::Rc8M => "Internal RC oscillator",
            Self::Rc128k => "Internal 128 kHz",
            Self::External => "External clock",
            Self::Crystal => "Crystal oscillator",
            Self::LowFreqCrystal => "32.768 kHz crystal",
            Self::Pll16M => "PLL (16 MHz)",
            Self::Rc6M4 => "ATtiny15 mode (6.4 MHz)",
            Self::RcHalf => "Internal RC (half frequency)",
        }
    }
}

pub struct ClassicSystemConfig {
    pub clkpr: u16,
    pub mcusr: u16,
    pub mcucr: u16,
    /// Plain MCUCR bits (PUD, and SE/SM/ISC0 on the ATtiny85).
    pub mcucr_plain: u8,
    /// (IVSEL, IVCE) when the device can move the vector table to the boot section.
    pub ivsel: Option<(u8, u8)>,
    /// (BODS, BODSE): BOD disable during sleep.
    pub bods: Option<(u8, u8)>,
    pub prr: u16,
    pub prr_mask: u8,
    pub osccal: u16,
    pub pllcsr: Option<u16>,
    /// CKSEL value -> clock source (values not listed are reserved: internal RC is used).
    pub cksel: Vec<(u8, ClockSource)>,
    /// XTAL1/CLKI and XTAL2 GPIOs.
    pub xtal1: Option<usize>,
    pub xtal2: Option<usize>,
    /// BODLEVEL value -> threshold (V); values not listed disable the BOD.
    pub bod_levels: Vec<(u8, f64)>,
}

const PORF: u8 = 0x01;
const EXTRF: u8 = 0x02;
const BORF: u8 = 0x04;
const WDRF: u8 = 0x08;
const CLKPCE: u8 = 0x80;
const PLLE: u8 = 0x02;
const PCKE: u8 = 0x04;
const PLOCK: u8 = 0x01;
const EV_PLL_LOCK: u8 = 0;
/// PLL lock time (Atmel-2586Q 6.2.2: typically 100 µs).
const PLL_LOCK_S: f64 = 100e-6;

pub struct ClassicSystem {
    c: ClassicSystemConfig,
    flags: u8,
    source: ClockSource,
    clkpce_until: u64,
    ivce_until: u64,
    bod_holding: bool,
}

impl ClassicSystem {
    pub fn new(c: ClassicSystemConfig) -> Self {
        Self { c, flags: 0, source: ClockSource::Rc8M, clkpce_until: 0, ivce_until: 0, bod_holding: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        let mut v = vec![(c.clkpr, 0), (c.mcusr, 0), (c.mcucr, 0), (c.prr, 0)];
        v.extend(c.pllcsr.map(|a| (a, 0)));
        v
    }

    fn base_hz(&self, cx: &Cx) -> f64 {
        let spec = cx.cpu.spec;
        match self.source {
            ClockSource::Rc8M => spec.clock.internal_hz,
            ClockSource::Rc128k => spec.clock.slow_hz,
            ClockSource::External | ClockSource::Crystal => cx.sys.ext_clock_hz,
            ClockSource::LowFreqCrystal => 32_768.0,
            ClockSource::Pll16M => 16e6,
            ClockSource::Rc6M4 => 6.4e6,
            ClockSource::RcHalf => spec.clock.internal_hz / 2.0,
        }
    }

    fn update_clock(&self, cx: &mut Cx) {
        let ps = (cx.cpu.data[self.c.clkpr as usize] & 0x0f).min(8);
        let hz = self.base_hz(cx) / (1u32 << ps) as f64;
        let now = cx.now();
        if cx.sys.clock.set_hz(hz, now) {
            cx.sys.events.push_back(Event::ClockChanged);
        }
    }

    fn bod_threshold(&self, cx: &Cx) -> Option<f64> {
        let lvl = cx.cpu.fuse_value("BODLEVEL")?;
        self.c.bod_levels.iter().find(|l| l.0 == lvl).map(|l| l.1)
    }

    fn evaluate_bod(&mut self, cx: &mut Cx) {
        let Some(th) = self.bod_threshold(cx) else {
            if self.bod_holding {
                self.bod_holding = false;
                cx.sys.brown_out = false;
                cx.sys.refresh_reset_held();
            }
            return;
        };
        let vcc = cx.sys.vcc;
        if !self.bod_holding && vcc < th {
            self.bod_holding = true;
            cx.sys.brown_out = true;
            cx.sys.refresh_reset_held();
            cx.sys.reset_request = Some(ResetSource::BrownOut);
            let now = cx.now();
            cx.sys.message(now, "warning", format!("Brown-out: VCC {vcc:.2} V is below the BOD level {th:.1} V - MCU held in reset"));
        } else if self.bod_holding && vcc >= th + 0.05 {
            self.bod_holding = false;
            cx.sys.brown_out = false;
            cx.sys.refresh_reset_held();
            let now = cx.now();
            cx.sys.message(now, "info", "Brown-out released: VCC back above the BOD level");
        }
    }

    fn boot_start(cx: &Cx) -> u32 {
        let spec = cx.cpu.spec;
        match (spec.boot.as_ref(), cx.cpu.fuse_value("BOOTSZ")) {
            (Some(b), Some(sz)) => cx.cpu.flash_words - b.sizes_words[(sz as usize).min(3)],
            _ => 0,
        }
    }

    /// Reserve the oscillator pins for the selected clock source.
    fn apply_xtal_pins(&self, cx: &mut Cx) {
        let (x1, x2) = match self.source {
            ClockSource::External => (true, false),
            ClockSource::Crystal | ClockSource::LowFreqCrystal => (true, true),
            _ => (false, false),
        };
        for (g, on, label) in [(self.c.xtal1, x1, "XTAL1"), (self.c.xtal2, x2, "XTAL2")] {
            if let Some(g) = g {
                let p = &mut cx.sys.pins[g];
                if p.reserved_by == "RESET" {
                    continue;
                }
                p.reserved = on;
                p.reserved_by = if on { label } else { "" };
            }
        }
    }

    fn pll_locked(&self, cx: &Cx) -> bool {
        self.c.pllcsr.is_some_and(|a| cx.cpu.data[a as usize] & PLOCK != 0)
    }
}

impl Peripheral for ClassicSystem {
    fn name(&self) -> &str {
        "SYSTEM"
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let c = &self.c;
        let a = addr as usize;
        let now = cx.now();
        if addr == c.clkpr {
            if v == CLKPCE {
                self.clkpce_until = now + 4;
            } else if now <= self.clkpce_until && v & CLKPCE == 0 {
                cx.cpu.data[a] = v & 0x0f;
                self.clkpce_until = 0;
                self.update_clock(cx);
            } else {
                cx.warn("clkpr", "CLKPR write ignored: write 0x80 (CLKPCE) first, then the prescaler within 4 cycles");
            }
        } else if addr == c.mcusr {
            self.flags &= v & 0x0f; // flags are cleared by writing 0
            cx.cpu.data[a] = self.flags;
        } else if addr == c.mcucr {
            let old = cx.cpu.data[a];
            let mut nv = (old & !c.mcucr_plain) | (v & c.mcucr_plain);
            if let Some((ivsel, ivce)) = c.ivsel {
                if v & ivce != 0 {
                    self.ivce_until = now + 4;
                } else if now <= self.ivce_until {
                    nv = (nv & !ivsel) | (v & ivsel);
                    self.ivce_until = 0;
                    cx.cpu.vector_base = if nv & ivsel != 0 { Self::boot_start(cx) } else { 0 };
                } else if (v ^ old) & ivsel != 0 {
                    cx.warn("ivsel", "MCUCR.IVSEL change ignored: write IVCE first, then IVSEL within 4 cycles");
                }
            }
            if let Some((bods, bodse)) = c.bods {
                nv = (nv & !(bods | bodse)) | (v & (bods | bodse));
            }
            cx.cpu.data[a] = nv;
            cx.sys.events.push_back(Event::RegWritten(addr));
        } else if addr == c.prr {
            let v = v & c.prr_mask;
            cx.cpu.data[a] = v;
            cx.sys.events.push_back(Event::PowerReduction(v));
        } else if Some(addr) == c.pllcsr {
            let old = cx.cpu.data[a];
            let forced = self.source == ClockSource::Pll16M;
            let mut nv = (v & 0x86) | (old & PLOCK);
            if forced {
                nv |= PLLE;
            }
            if nv & PLLE == 0 {
                nv &= !PLOCK;
                cx.cancel(EV_PLL_LOCK);
            } else if old & PLLE == 0 {
                let at = cx.sys.clock.cycle_at(cx.time_seconds() + PLL_LOCK_S).max(now + 1);
                cx.schedule(EV_PLL_LOCK, at);
            }
            cx.cpu.data[a] = nv;
            if (old ^ nv) & (PCKE | 0x80 | PLOCK) != 0 {
                cx.sys.events.push_back(Event::RegWritten(addr));
            }
        }
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        if let Some(a) = self.c.pllcsr {
            if cx.cpu.data[a as usize] & PLLE != 0 {
                cx.cpu.data[a as usize] |= PLOCK;
                cx.sys.events.push_back(Event::RegWritten(a));
            }
        }
    }

    fn on_ext_clock(&mut self, cx: &mut Cx) {
        if matches!(self.source, ClockSource::External | ClockSource::Crystal) {
            self.update_clock(cx);
        }
    }

    fn on_vcc_change(&mut self, cx: &mut Cx) {
        self.evaluate_bod(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        let flag = match cx.sys.last_reset {
            ResetSource::PowerOn => PORF,
            ResetSource::BrownOut => BORF,
            ResetSource::External => EXTRF,
            ResetSource::Watchdog => WDRF,
            ResetSource::Debugger => 0,
        };
        self.flags = if cx.sys.last_reset == ResetSource::PowerOn { PORF } else { self.flags | flag };
        cx.cpu.data[self.c.mcusr as usize] = self.flags;
        cx.cpu.data[self.c.osccal as usize] = cx.cpu.spec.calibration;
        let cksel = cx.cpu.fuse_value("CKSEL").unwrap_or(2);
        self.source = self.c.cksel.iter().find(|e| e.0 == cksel).map(|e| e.1).unwrap_or(ClockSource::Rc8M);
        cx.cpu.data[self.c.clkpr as usize] = if cx.fuse_programmed("CKDIV8") { 3 } else { 0 };
        self.clkpce_until = 0;
        self.ivce_until = 0;
        if let Some(a) = self.c.pllcsr {
            // The PLL runs (and is locked) when it clocks the CPU.
            cx.cpu.data[a as usize] = if self.source == ClockSource::Pll16M { PLLE | PLOCK } else { 0 };
        }
        self.apply_xtal_pins(cx);
        self.update_clock(cx);
        self.evaluate_bod(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let hz = cx.sys.clock.hz;
        let ps = cx.cpu.data[self.c.clkpr as usize] & 0x0f;
        let mut v = vec![
            ("CPU clock".into(), if hz >= 1e6 { format!("{} MHz", hz / 1e6) } else { format!("{} kHz", hz / 1e3) }),
            ("Clock source".into(), self.source.label().into()),
            ("Prescaler".into(), format!("/{}", 1u32 << ps.min(8))),
            ("VCC (V)".into(), format!("{:.2}", cx.sys.vcc)),
            ("Brown-out level".into(), self.bod_threshold(cx).map(|t| format!("{t:.1} V")).unwrap_or_else(|| "Disabled".into())),
            ("Last reset".into(), cx.sys.last_reset.label().into()),
        ];
        if self.c.pllcsr.is_some() {
            v.push(("PLL".into(), if self.pll_locked(cx) { "Locked (64 MHz)" } else { "Off" }.into()));
        }
        v
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
