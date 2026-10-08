//! SPI: master transfers clocked on SCK with the four CPOL/CPHA modes, MSB/LSB first, the
//! fosc/2..fosc/128 clock rates, write collision and mode-fault (SS) detection, and slave mode
//! driven by external SCK edges. Bits travel on MOSI/MISO/SCK so the waveform shows them.
//!
//! Source: DS40002061B section 19 (SPI).

use crate::avr::machine::{Cx, Peripheral};

#[derive(Clone)]
pub struct SpiConfig {
    pub spcr: u16,
    pub spsr: u16,
    pub spdr: u16,
    pub ss_gpio: usize,
    pub mosi_gpio: usize,
    pub miso_gpio: usize,
    pub sck_gpio: usize,
    pub vector: u8,
    pub prr_mask: u8,
}

const SPIE: u8 = 0x80;
const SPE: u8 = 0x40;
const DORD: u8 = 0x20;
const MSTR: u8 = 0x10;
const CPOL: u8 = 0x08;
const CPHA: u8 = 0x04;
const SPIF: u8 = 0x80;
const WCOL: u8 = 0x40;
const SPI2X: u8 = 0x01;
const EV_EDGE: u8 = 0;
const DIV: [u64; 4] = [4, 16, 64, 128];

pub struct Spi {
    c: SpiConfig,
    spcr: u8,
    spsr: u8,
    /// Data being shifted (transmit side).
    shift: u8,
    /// Received byte (read through SPDR).
    rx: u8,
    /// Byte being assembled from MISO/MOSI.
    rx_shift: u8,
    /// Master: SCK edges done in the current transfer (0..16), None when idle.
    edge: Option<u8>,
    /// Slave: sampling edges seen in the current byte.
    slave_bits: u8,
    /// SPSR read while SPIF was set (the next SPDR access clears SPIF).
    spif_seen: bool,
}

impl Spi {
    pub fn new(c: SpiConfig) -> Self {
        Self { c, spcr: 0, spsr: 0, shift: 0, rx: 0, rx_shift: 0, edge: None, slave_bits: 0, spif_seen: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.spcr, 0), (self.c.spsr, 0), (self.c.spdr, 0)]
    }

    fn master(&self) -> bool {
        self.spcr & (SPE | MSTR) == SPE | MSTR
    }

    fn slave(&self) -> bool {
        self.spcr & (SPE | MSTR) == SPE
    }

    fn half_period(&self) -> u64 {
        let d = DIV[(self.spcr & 3) as usize] / if self.spsr & SPI2X != 0 { 2 } else { 1 };
        (d / 2).max(1)
    }

    /// Next bit to transmit (MSB or LSB first).
    fn out_bit(&self, n: u8) -> u8 {
        if self.spcr & DORD != 0 { (self.shift >> n) & 1 } else { (self.shift >> (7 - n)) & 1 }
    }

    fn take_bit(&mut self, n: u8, b: u8) {
        if self.spcr & DORD != 0 {
            self.rx_shift |= b << n;
        } else {
            self.rx_shift |= b << (7 - n);
        }
    }

    fn drive(&self, gpio: usize, en: bool, val: u8, cx: &mut Cx) {
        let now = cx.now();
        let p = &mut cx.sys.pins[gpio];
        let (en, val) = (en as u8, val);
        if p.ov_enable != en || p.ov_value != val {
            p.ov_enable = en;
            p.ov_value = val;
            cx.sys.update_pin(gpio, now);
        }
    }

    fn force_input(&self, gpio: usize, on: bool, cx: &mut Cx) {
        let now = cx.now();
        let p = &mut cx.sys.pins[gpio];
        let on = on as u8;
        if p.ddoe != on || p.ddov != 0 {
            p.ddoe = on;
            p.ddov = 0;
            cx.sys.update_pin(gpio, now);
        }
    }

    /// Port overrides for the current mode (DS40002061B table 14-6).
    fn apply_pins(&self, cx: &mut Cx) {
        let m = self.master();
        let s = self.slave();
        let cpol = (self.spcr & CPOL != 0) as u8;
        let sck = match self.edge {
            Some(e) => cpol ^ (e & 1),
            None => cpol,
        };
        self.drive(self.c.sck_gpio, m, sck, cx);
        let mosi = if self.edge.is_some() { self.cur_out() } else { 1 };
        self.drive(self.c.mosi_gpio, m, mosi, cx);
        self.force_input(self.c.miso_gpio, m, cx);
        self.force_input(self.c.mosi_gpio, s, cx);
        self.force_input(self.c.sck_gpio, s, cx);
        self.force_input(self.c.ss_gpio, s, cx);
        let ss_low = cx.sys.pins[self.c.ss_gpio].level == 0;
        let miso = if s && ss_low { self.out_bit(self.slave_bits.min(7)) } else { 0 };
        self.drive(self.c.miso_gpio, s && ss_low, miso, cx);
    }

    /// Bit currently on the data output (CPHA decides whether it changes on the leading edge).
    fn cur_out(&self) -> u8 {
        let e = self.edge.unwrap_or(0);
        let idx = if self.spcr & CPHA == 0 { e / 2 } else { e.saturating_sub(1) / 2 };
        self.out_bit(idx.min(7))
    }

    fn complete(&mut self, cx: &mut Cx) {
        self.rx = self.rx_shift;
        self.spsr |= SPIF;
        self.update_irq(cx);
    }

    fn update_irq(&self, cx: &mut Cx) {
        cx.cpu.set_irq(self.c.vector, self.spcr & (SPIE | SPE) == SPIE | SPE && self.spsr & SPIF != 0);
    }

    fn spdr_access(&mut self) {
        if self.spif_seen {
            self.spsr &= !(SPIF | WCOL);
            self.spif_seen = false;
        }
    }
}

impl Peripheral for Spi {
    fn name(&self) -> &str {
        "SPI"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        if addr == self.c.spsr {
            self.spif_seen = self.spsr & SPIF != 0;
            return self.spsr;
        }
        if addr == self.c.spdr {
            self.spdr_access();
            self.update_irq(cx);
            return self.rx;
        }
        self.spcr
    }

    fn peek(&mut self, addr: u16, _cx: &mut Cx) -> u8 {
        if addr == self.c.spsr {
            self.spsr
        } else if addr == self.c.spdr {
            self.rx
        } else {
            self.spcr
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        if addr == self.c.spcr {
            self.spcr = v;
            if v & SPE == 0 {
                self.edge = None;
                cx.cancel(EV_EDGE);
            }
        } else if addr == self.c.spsr {
            self.spsr = (self.spsr & !SPI2X) | (v & SPI2X);
        } else {
            self.spdr_access();
            if self.edge.is_some() {
                self.spsr |= WCOL;
            } else {
                self.shift = v;
                self.rx_shift = 0;
                self.slave_bits = 0;
                if self.master() {
                    self.edge = Some(0);
                    let at = cx.now() + self.half_period();
                    cx.schedule(EV_EDGE, at);
                }
            }
        }
        self.apply_pins(cx);
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        let Some(e) = self.edge else { return };
        let e = e + 1;
        self.edge = Some(e);
        let leading = e & 1 == 1;
        let sample = leading == (self.spcr & CPHA == 0);
        if sample {
            let b = cx.sys.pins[self.c.miso_gpio].level;
            self.take_bit((e - 1) / 2, b);
        }
        if e >= 16 {
            self.edge = None;
            self.complete(cx);
        } else {
            let at = cx.now() + self.half_period();
            cx.schedule(EV_EDGE, at);
        }
        self.apply_pins(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, _cycle: u64, cx: &mut Cx) {
        let pin = pin as usize;
        if pin == self.c.ss_gpio && level == 0 && self.master() && cx.sys.pins[pin].effective_dir() == 0 {
            // Mode fault: SS driven low while it is an input in master mode.
            self.spcr &= !MSTR;
            self.spsr |= SPIF;
            self.edge = None;
            cx.cancel(EV_EDGE);
            self.apply_pins(cx);
            self.update_irq(cx);
            return;
        }
        if !self.slave() {
            return;
        }
        if pin == self.c.ss_gpio {
            self.slave_bits = 0;
            self.rx_shift = 0;
            self.apply_pins(cx);
            return;
        }
        if pin != self.c.sck_gpio || cx.sys.pins[self.c.ss_gpio].level != 0 {
            return;
        }
        let cpol = (self.spcr & CPOL != 0) as u8;
        let leading = level != cpol;
        let sample = leading == (self.spcr & CPHA == 0);
        if sample {
            let b = cx.sys.pins[self.c.mosi_gpio].level;
            self.take_bit(self.slave_bits, b);
            self.slave_bits += 1;
            if self.slave_bits == 8 {
                self.slave_bits = 0;
                self.complete(cx);
                self.rx_shift = 0;
            }
        }
        self.apply_pins(cx);
    }

    fn ack(&mut self, _vector: u8, cx: &mut Cx) {
        self.spsr &= !SPIF;
        self.update_irq(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_EDGE);
        *self = Spi::new(self.c.clone());
        self.apply_pins(cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let mode = ((self.spcr & CPOL != 0) as u8) << 1 | (self.spcr & CPHA != 0) as u8;
        let role = if self.master() { "Master" } else if self.slave() { "Slave" } else { "Disabled" };
        let rate = cx.sys.clock.hz / (self.half_period() * 2) as f64;
        vec![
            ("Role".into(), role.into()),
            ("Mode".into(), format!("SPI mode {mode}, {}", if self.spcr & DORD != 0 { "LSB first" } else { "MSB first" })),
            ("SCK".into(), format!("{:.0} Hz", rate)),
            ("State".into(), if self.edge.is_some() { "Transferring" } else { "Idle" }.into()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
