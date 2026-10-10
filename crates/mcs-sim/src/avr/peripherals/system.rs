//! System blocks of the AVRrc tinies: configuration change protection, clock source/prescaler,
//! sleep control, reset flags, power reduction, VCC level monitor, NVM controller registers,
//! and the watchdog timer.

use crate::avr::machine::{Cx, Event, Peripheral, ResetSource};

pub struct SystemConfig {
    pub ccp: u16,
    pub clkmsr: u16,
    pub clkpsr: u16,
    pub osccal: u16,
    pub smcr: u16,
    pub rstflr: u16,
    pub prr: u16,
    pub vlmcsr: u16,
    pub vlm_vector: u8,
    pub nvmcsr: u16,
    pub nvmcmd: u16,
}

const CCP_SIGNATURE: u8 = 0xd8;
const PORF: u8 = 0x01;
const EXTRF: u8 = 0x02;
const WDRF: u8 = 0x08;
/// VLM trigger levels (typical values, datasheet electrical characteristics).
const VLM_LEVELS: [f64; 5] = [0.0, 1.4, 1.6, 2.5, 3.7];

pub struct System {
    c: SystemConfig,
    /// Reset flags (kept here because they survive non power-on resets).
    flags: u8,
}

impl System {
    pub fn new(c: SystemConfig) -> Self {
        Self { c, flags: 0 }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        vec![(c.ccp, 0), (c.clkmsr, 0), (c.clkpsr, 0), (c.rstflr, 0), (c.prr, 0), (c.vlmcsr, 0), (c.nvmcsr, 0), (c.smcr, 0)]
    }

    fn update_clock(&self, cx: &mut Cx) {
        let d = &cx.cpu.data;
        let spec = cx.cpu.spec;
        let base = match d[self.c.clkmsr as usize] & 3 {
            0 => spec.clock.internal_hz,
            1 => spec.clock.slow_hz,
            _ => cx.sys.ext_clock_hz,
        };
        let ps = (d[self.c.clkpsr as usize] & 0x0f).min(8);
        let now = cx.now();
        if cx.sys.clock.set_hz(base / (1u32 << ps) as f64, now) {
            cx.sys.events.push_back(Event::ClockChanged);
        }
    }

    fn vlm_low(&self, cx: &Cx) -> bool {
        let lvl = (cx.cpu.data[self.c.vlmcsr as usize] & 7) as usize;
        lvl > 0 && lvl < VLM_LEVELS.len() && cx.sys.vcc < VLM_LEVELS[lvl]
    }

    fn vlm_value(&self, cx: &Cx) -> u8 {
        let v = cx.cpu.data[self.c.vlmcsr as usize] & 0x47;
        v | if self.vlm_low(cx) && v & 7 >= 3 { 0x80 } else { 0 }
    }

    fn evaluate_vlm(&self, cx: &mut Cx) {
        let reg = cx.cpu.data[self.c.vlmcsr as usize];
        let lvl = reg & 7;
        let low = self.vlm_low(cx);
        if low && (lvl == 1 || lvl == 2) {
            cx.sys.reset_request = Some(ResetSource::BrownOut);
            return;
        }
        cx.cpu.set_irq(self.c.vlm_vector, low && lvl >= 3 && reg & 0x40 != 0);
    }
}

impl Peripheral for System {
    fn name(&self) -> &str {
        "SYSTEM"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        if addr == self.c.ccp || addr == self.c.nvmcsr {
            0
        } else if addr == self.c.vlmcsr {
            self.vlm_value(cx)
        } else {
            cx.cpu.data[addr as usize]
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let c = &self.c;
        let a = addr as usize;
        if addr == c.ccp {
            if v == CCP_SIGNATURE {
                cx.sys.ccp_until = cx.now() + 4;
            }
        } else if addr == c.clkmsr || addr == c.clkpsr {
            if cx.now() > cx.sys.ccp_until {
                let name = if addr == c.clkmsr { "CLKMSR" } else { "CLKPSR" };
                cx.warn(name, format!("{name} write ignored: Configuration Change Protection not unlocked (write 0xD8 to CCP first)"));
                return;
            }
            cx.cpu.data[a] = if addr == c.clkmsr { v & 3 } else { v & 0x0f };
            self.update_clock(cx);
        } else if addr == c.rstflr {
            self.flags &= v & 0x0b; // flags are cleared by writing 0
            cx.cpu.data[a] = self.flags;
        } else if addr == c.prr {
            cx.cpu.data[a] = v & 0x03;
            cx.sys.events.push_back(Event::PowerReduction(v & 0x03));
        } else if addr == c.vlmcsr {
            cx.cpu.data[a] = v & 0x47;
            self.evaluate_vlm(cx);
        } else if addr == c.nvmcsr {
            // NVMBSY is read-only and always 0 (no self-programming).
        } else if addr == c.smcr {
            cx.cpu.data[a] = v & 0x0f;
        }
    }

    fn on_vcc_change(&mut self, cx: &mut Cx) {
        self.evaluate_vlm(cx);
    }

    fn on_ext_clock(&mut self, cx: &mut Cx) {
        self.update_clock(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        let flag = match cx.sys.last_reset {
            ResetSource::PowerOn | ResetSource::BrownOut => PORF,
            ResetSource::External => EXTRF,
            ResetSource::Watchdog => WDRF,
            ResetSource::Debugger => 0,
        };
        self.flags = if cx.sys.last_reset == ResetSource::PowerOn { PORF } else { self.flags | flag };
        cx.cpu.data[self.c.rstflr as usize] = self.flags;
        cx.cpu.data[self.c.osccal as usize] = cx.cpu.spec.calibration;
        cx.sys.ccp_until = 0;
        self.update_clock(cx);
        self.evaluate_vlm(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let hz = cx.sys.clock.hz;
        let source = match cx.cpu.data[self.c.clkmsr as usize] & 3 {
            0 => "Internal 8 MHz",
            1 => "Internal 128 kHz",
            _ => "External (CLKI)",
        };
        vec![
            ("CPU clock".into(), if hz >= 1e6 { format!("{} MHz", hz / 1e6) } else { format!("{} kHz", hz / 1e3) }),
            ("Clock source".into(), source.into()),
            ("Prescaler".into(), format!("/{}", 1u32 << (cx.cpu.data[self.c.clkpsr as usize] & 0x0f).min(8))),
            ("CCP unlocked".into(), if cx.now() <= cx.sys.ccp_until { "Yes" } else { "No" }.into()),
            ("VCC (V)".into(), format!("{:.2}", cx.sys.vcc)),
            ("Last reset".into(), cx.sys.last_reset.label().into()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub struct WatchdogConfig {
    pub wdtcsr: u16,
    /// Reset flag register (RSTFLR / MCUSR).
    pub rstflr: u16,
    /// Watchdog interrupt vector (unused when `legacy`).
    pub vector: u8,
    /// Classic AVRs protect WDE/WDP with the WDCE timed sequence instead of CCP.
    pub wdce: bool,
    /// ATmega8/16/32 watchdog (WDTCR): no interrupt mode (WDIF/WDIE) and no WDP3, the prescaler
    /// can be changed at any time, only clearing WDE needs the WDCE|WDE timed sequence; the
    /// time-outs are the same as the classic parts' (16.3 ms * 2^WDP at 5 V).
    pub legacy: bool,
}

const WDIF: u8 = 0x80;
const WDIE: u8 = 0x40;
const WDE: u8 = 0x08;
const WDT_OSC_HZ: f64 = 128_000.0;
const EV_TIMEOUT: u8 = 0;

const WDCE: u8 = 0x10;

pub struct Watchdog {
    c: WatchdogConfig,
    start_time: f64,
    /// End of the WDCE change-enable window (inclusive cycle).
    wdce_until: u64,
}

impl Watchdog {
    pub fn new(c: WatchdogConfig) -> Self {
        Self { c, start_time: 0.0, wdce_until: 0 }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.wdtcsr, if self.c.legacy { 0 } else { WDIF })]
    }

    fn reg(&self, cx: &Cx) -> u8 {
        cx.cpu.data[self.c.wdtcsr as usize]
    }

    /// WDE is forced on while WDRF is set or the WDTON fuse is programmed.
    fn refresh_wde(&mut self, cx: &mut Cx) {
        if (!self.c.legacy && cx.cpu.data[self.c.rstflr as usize] & WDRF != 0) || cx.fuse_programmed("WDTON") {
            cx.cpu.data[self.c.wdtcsr as usize] |= WDE;
        }
    }

    fn period_seconds(&self, cx: &Cx) -> f64 {
        let v = self.reg(cx);
        let wdp = ((v & 7) | if self.c.legacy { 0 } else { (v >> 2) & 8 }).min(9);
        2048.0 * (1u32 << wdp) as f64 / WDT_OSC_HZ
    }

    fn active(&self, cx: &Cx) -> bool {
        self.reg(cx) & (WDE | WDIE) != 0
    }

    fn restart(&mut self, cx: &mut Cx) {
        self.start_time = cx.time_seconds();
        self.schedule(cx);
    }

    fn schedule(&mut self, cx: &mut Cx) {
        if !self.active(cx) {
            cx.cancel(EV_TIMEOUT);
            return;
        }
        let target = cx.sys.clock.cycle_at(self.start_time + self.period_seconds(cx));
        let at = target.max(cx.now());
        cx.schedule(EV_TIMEOUT, at);
    }

    /// WDTCR of the legacy parts (Atmel-2466T "Watchdog Timer Control Register").
    fn write_legacy(&mut self, v: u8, cx: &mut Cx) {
        let a = self.c.wdtcsr as usize;
        let old = cx.cpu.data[a];
        let now = cx.now();
        let unlocked = now <= self.wdce_until && v & WDCE == 0;
        let mut nv = v & 0x07; // WDP2:0 are freely writable
        if v & WDE != 0 || (old & WDE != 0 && !unlocked) {
            nv |= WDE; // setting is always allowed; clearing needs the sequence
            if v & WDE == 0 {
                cx.warn("wdce-wdt", "WDTCR: clearing WDE needs the timed sequence (write WDCE|WDE, then WDE = 0 within 4 cycles)");
            }
        }
        if unlocked {
            self.wdce_until = 0;
        } else if v & (WDCE | WDE) == WDCE | WDE {
            self.wdce_until = now + 4;
        }
        // WDE is forced on while the WDTON fuse is programmed.
        if cx.fuse_programmed("WDTON") {
            nv |= WDE;
        }
        cx.cpu.data[a] = nv;
        self.restart(cx);
    }

    fn update_irq(&self, cx: &mut Cx) {
        if self.c.legacy {
            return; // no watchdog interrupt (`vector` is unused)
        }
        let v = self.reg(cx);
        cx.cpu.set_irq(self.c.vector, v & (WDIF | WDIE) == WDIF | WDIE);
    }
}

impl Peripheral for Watchdog {
    fn name(&self) -> &str {
        "WDT"
    }

    fn write(&mut self, _addr: u16, v: u8, cx: &mut Cx) {
        if self.c.legacy {
            return self.write_legacy(v, cx);
        }
        let a = self.c.wdtcsr as usize;
        let old = cx.cpu.data[a];
        let mut nv = (old & WDIF & !v) | (v & WDIE);
        const PROTECTED: u8 = 0x2f; // WDP3, WDE, WDP2:0
        let now = cx.now();
        let unlocked = if self.c.wdce { now <= self.wdce_until && v & WDCE == 0 } else { now <= cx.sys.ccp_until };
        if unlocked {
            nv |= v & PROTECTED;
            self.wdce_until = 0;
        } else {
            nv |= old & PROTECTED;
            if v & WDE != 0 {
                nv |= WDE; // enabling is always allowed
            }
            if self.c.wdce && v & (WDCE | WDE) == WDCE | WDE {
                // Timed sequence step 1: changes are allowed for the next four cycles.
                self.wdce_until = now + 4;
            } else {
                let blocked = (v ^ old) & 0x27 != 0 || (old & WDE != 0 && v & WDE == 0);
                if blocked {
                    if self.c.wdce {
                        cx.warn("wdce-wdt", "WDTCSR: clearing WDE or changing WDP needs the timed sequence (write WDCE|WDE, then the new value within 4 cycles)");
                    } else {
                        cx.warn("ccp-wdt", "WDTCSR: clearing WDE or changing WDP requires the CCP unlock sequence (0xD8 to CCP)");
                    }
                }
            }
        }
        cx.cpu.data[a] = nv;
        self.refresh_wde(cx);
        self.restart(cx);
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        let v = self.reg(cx);
        if v & WDIE != 0 {
            cx.cpu.data[self.c.wdtcsr as usize] |= WDIF;
            self.update_irq(cx);
            self.restart(cx);
        } else if v & WDE != 0 {
            let now = cx.now();
            cx.sys.message(now, "warning", "Watchdog timeout: system reset");
            cx.sys.reset_request = Some(ResetSource::Watchdog);
        }
    }

    fn ack(&mut self, _vector: u8, cx: &mut Cx) {
        let a = self.c.wdtcsr as usize;
        cx.cpu.data[a] &= !WDIF;
        // Interrupt + system reset mode: the interrupt switches the WDT to system reset mode.
        if cx.cpu.data[a] & WDE != 0 {
            cx.cpu.data[a] &= !WDIE;
        }
        self.update_irq(cx);
    }

    fn on_wdr(&mut self, cx: &mut Cx) {
        self.restart(cx);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.schedule(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.start_time = cx.time_seconds();
        self.wdce_until = 0;
        self.refresh_wde(cx);
        self.schedule(cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let v = self.reg(cx);
        let mode = match (v & WDE != 0, v & WDIE != 0) {
            (true, true) => "Interrupt + Reset",
            (true, false) => "System Reset",
            (false, true) => "Interrupt",
            _ => "Stopped",
        };
        let period = self.period_seconds(cx);
        let remaining = if self.active(cx) { (self.start_time + period - cx.time_seconds()).max(0.0) } else { 0.0 };
        vec![
            ("Mode".into(), mode.into()),
            ("Timeout (ms)".into(), format!("{:.0}", period * 1000.0)),
            ("Remaining (ms)".into(), format!("{:.2}", remaining * 1000.0)),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
