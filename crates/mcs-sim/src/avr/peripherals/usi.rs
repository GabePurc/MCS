//! USI (Universal Serial Interface, ATtiny25/45/85): 8-bit shift register + 4-bit counter
//! clocked by software strobes (USICLK / USITC), Timer0 compare match or external USCK edges;
//! three-wire mode output on DO, two-wire start/stop condition detection, counter overflow
//! interrupt with the USIBR buffer.
//!
//! Source: Atmel-2586Q section 15 (USI).

use crate::avr::machine::{Cx, Event, Peripheral, Trigger};

#[derive(Clone)]
pub struct UsiConfig {
    pub usicr: u16,
    pub usisr: u16,
    pub usidr: u16,
    pub usibr: u16,
    /// PORTB / DDRB (USITC toggles the USCK port bit).
    pub port: u16,
    pub di_gpio: usize,
    pub do_gpio: usize,
    pub usck_gpio: usize,
    /// USCK bit in PORTB.
    pub usck_bit: u8,
    pub v_start: u8,
    pub v_ovf: u8,
    pub prr_mask: u8,
}

const USISIE: u8 = 0x80;
const USIOIE: u8 = 0x40;
const USICLK: u8 = 0x02;
const USITC: u8 = 0x01;
const USISIF: u8 = 0x80;
const USIOIF: u8 = 0x40;
const USIPF: u8 = 0x20;

pub struct Usi {
    c: UsiConfig,
    usicr: u8,
    flags: u8,
    counter: u8,
    data: u8,
}

impl Usi {
    pub fn new(c: UsiConfig) -> Self {
        Self { c, usicr: 0, flags: 0, counter: 0, data: 0 }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.usicr, 0), (self.c.usisr, 0), (self.c.usidr, 0), (self.c.usibr, 0)]
    }

    fn wire_mode(&self) -> u8 {
        (self.usicr >> 4) & 3
    }

    fn clock_select(&self) -> u8 {
        (self.usicr >> 2) & 3
    }

    fn shift(&mut self, cx: &mut Cx) {
        let di = cx.sys.pins[self.c.di_gpio].level;
        self.data = (self.data << 1) | di;
        self.apply_do(cx);
    }

    fn count(&mut self, cx: &mut Cx) {
        self.counter = (self.counter + 1) & 0x0f;
        if self.counter == 0 {
            self.flags |= USIOIF;
            cx.cpu.data[self.c.usibr as usize] = self.data;
        }
        self.update_irq(cx);
    }

    /// DO follows the MSB of the data register in three-wire mode; SDA (two-wire) is pulled
    /// low by a 0 MSB.
    fn apply_do(&self, cx: &mut Cx) {
        let now = cx.now();
        let wm = self.wire_mode();
        let msb = self.data >> 7;
        let (gpio, en) = match wm {
            1 => (self.c.do_gpio, true),
            2 | 3 => (self.c.di_gpio, true),
            _ => (self.c.do_gpio, false),
        };
        // Release whichever pin is not used in the current mode.
        for g in [self.c.do_gpio, self.c.di_gpio] {
            if g != gpio || !en {
                let p = &mut cx.sys.pins[g];
                if p.ov_enable != 0 {
                    p.ov_enable = 0;
                    cx.sys.update_pin(g, now);
                }
            }
        }
        if en {
            let p = &mut cx.sys.pins[gpio];
            if p.ov_enable != 1 || p.ov_value != msb {
                p.ov_enable = 1;
                p.ov_value = msb;
                cx.sys.update_pin(gpio, now);
            }
        }
    }

    fn update_irq(&self, cx: &mut Cx) {
        cx.cpu.set_irq(self.c.v_ovf, self.usicr & USIOIE != 0 && self.flags & USIOIF != 0);
        cx.cpu.set_irq(self.c.v_start, self.usicr & USISIE != 0 && self.flags & USISIF != 0);
    }
}

impl Peripheral for Usi {
    fn name(&self) -> &str {
        "USI"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let c = &self.c;
        if addr == c.usicr {
            self.usicr & !(USICLK | USITC)
        } else if addr == c.usisr {
            self.flags | self.counter
        } else if addr == c.usidr {
            self.data
        } else {
            cx.cpu.data[addr as usize]
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let c = self.c.clone();
        if addr == c.usidr {
            self.data = v;
            self.apply_do(cx);
        } else if addr == c.usisr {
            self.flags &= !(v & (USISIF | USIOIF | USIPF)); // write one to clear
            self.counter = v & 0x0f;
        } else if addr == c.usicr {
            // USICLK is a strobe with USICS = 00 and selects the counter clock otherwise.
            self.usicr = v & !USITC;
            let cs = self.clock_select();
            if v & USICLK != 0 && cs == 0 {
                self.shift(cx);
                self.count(cx);
            }
            if v & USITC != 0 {
                // Toggle the USCK port bit; with USICS1 = 1 and USICLK = 1 this strobe also
                // clocks the counter (the data register follows the resulting pin edge).
                let a = c.port as usize;
                cx.cpu.data[a] ^= c.usck_bit;
                cx.sys.events.push_back(Event::RegWritten(c.port));
                if cs >= 2 && v & USICLK != 0 {
                    self.count(cx);
                }
            }
            self.apply_do(cx);
        }
        self.update_irq(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, _cycle: u64, cx: &mut Cx) {
        let pin = pin as usize;
        let wm = self.wire_mode();
        if wm >= 2 && pin == self.c.di_gpio && cx.sys.pins[self.c.usck_gpio].level == 1 {
            // SDA edge while SCL is high: start (falling) or stop (rising) condition.
            self.flags |= if level == 0 { USISIF } else { USIPF };
            if level == 0 {
                self.counter = 0;
            }
            self.update_irq(cx);
        }
        let cs = self.clock_select();
        if pin != self.c.usck_gpio || cs < 2 {
            return;
        }
        // USICS0 = 0: shift on the positive edge, 1: on the negative edge.
        let shift_edge = if cs == 2 { 1 } else { 0 };
        if level == shift_edge {
            self.shift(cx);
        }
        if self.usicr & USICLK == 0 {
            self.count(cx); // external clock: the counter counts both edges
        }
    }

    fn on_trigger(&mut self, trigger: Trigger, _value: u8, _cycle: u64, cx: &mut Cx) {
        if trigger == Trigger::TimerCompA(0) && self.clock_select() == 1 {
            self.shift(cx);
            self.count(cx);
        }
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        if vector == self.c.v_start {
            self.flags &= !USISIF;
        } else {
            self.flags &= !USIOIF;
        }
        self.update_irq(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        *self = Usi::new(self.c.clone());
        self.apply_do(cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, _cx: &mut Cx) -> Vec<(String, String)> {
        let mode = ["Disabled", "Three-wire", "Two-wire", "Two-wire (SCL hold)"][self.wire_mode() as usize];
        let clk = ["Software (USICLK)", "Timer0 compare match", "External, positive edge", "External, negative edge"][self.clock_select() as usize];
        vec![
            ("Mode".into(), mode.into()),
            ("Clock".into(), clk.into()),
            ("Data".into(), format!("0x{:02X}", self.data)),
            ("Counter".into(), self.counter.to_string()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
