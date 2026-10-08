//! TWI (I²C) master on an empty bus: START / repeated START, address and data bytes with the
//! bit-rate timing (SCL = F_CPU / (16 + 2·TWBR·4^TWPS)), STOP, TWINT / TWWC handling and the
//! status codes the hardware reports when no device acknowledges (no slaves are attached to the
//! simulated bus, so every address is NACKed).
//!
//! Source: DS40002061B section 21 (2-wire serial interface, tables 21-3 and 21-4).

use crate::avr::machine::{Cx, Peripheral};

#[derive(Clone)]
pub struct TwiConfig {
    pub twbr: u16,
    pub twsr: u16,
    pub twar: u16,
    pub twdr: u16,
    pub twcr: u16,
    pub twamr: u16,
    pub vector: u8,
    pub prr_mask: u8,
}

const TWINT: u8 = 0x80;
const TWSTA: u8 = 0x20;
const TWSTO: u8 = 0x10;
const TWWC: u8 = 0x08;
const TWEN: u8 = 0x04;
const TWIE: u8 = 0x01;
const EV_DONE: u8 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Bus {
    Idle,
    /// START sent, waiting for SLA+R/W.
    Started,
    /// Address NACKed: only STOP or repeated START make sense.
    Nacked,
}

pub struct Twi {
    c: TwiConfig,
    twcr: u8,
    status: u8,
    bus: Bus,
    /// Status to report when the pending action completes.
    next: Option<(u8, Bus)>,
}

impl Twi {
    pub fn new(c: TwiConfig) -> Self {
        Self { c, twcr: 0, status: 0xf8, bus: Bus::Idle, next: None }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.twsr, 0), (self.c.twcr, 0), (self.c.twdr, 0)]
    }

    /// CPU cycles per SCL period.
    fn scl_cycles(&self, cx: &Cx) -> u64 {
        let twbr = cx.cpu.data[self.c.twbr as usize] as u64;
        let ps = 4u64.pow((cx.cpu.data[self.c.twsr as usize] & 3) as u32);
        16 + 2 * twbr * ps
    }

    fn update_irq(&self, cx: &mut Cx) {
        cx.cpu.set_irq(self.c.vector, self.twcr & (TWINT | TWIE | TWEN) == TWINT | TWIE | TWEN);
    }

    fn act(&mut self, cx: &mut Cx, status: u8, bus: Bus, scl_periods: u64) {
        self.next = Some((status, bus));
        let at = cx.now() + self.scl_cycles(cx) * scl_periods;
        cx.schedule(EV_DONE, at);
    }
}

impl Peripheral for Twi {
    fn name(&self) -> &str {
        "TWI"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        if addr == self.c.twcr {
            self.twcr
        } else if addr == self.c.twsr {
            self.status | (cx.cpu.data[addr as usize] & 3)
        } else {
            cx.cpu.data[addr as usize]
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let a = addr as usize;
        if addr == self.c.twsr {
            cx.cpu.data[a] = v & 3; // prescaler bits only
            return;
        }
        if addr == self.c.twdr {
            if self.twcr & TWINT == 0 && self.next.is_some() {
                self.twcr |= TWWC;
            } else {
                cx.cpu.data[a] = v;
                self.twcr &= !TWWC;
            }
            return;
        }
        // TWCR: writing TWINT = 1 clears the flag and starts the requested action.
        let clear = v & TWINT != 0;
        self.twcr = (self.twcr & (TWINT | TWWC)) | (v & !(TWINT | TWWC));
        if clear {
            self.twcr &= !TWINT;
        }
        if v & TWEN == 0 {
            self.bus = Bus::Idle;
            self.next = None;
            cx.cancel(EV_DONE);
            self.update_irq(cx);
            return;
        }
        if clear && self.next.is_none() {
            if v & TWSTA != 0 {
                let code = if self.bus == Bus::Idle { 0x08 } else { 0x10 };
                self.act(cx, code, Bus::Started, 1);
            } else if v & TWSTO != 0 {
                // STOP: the bus is released; TWINT is not set.
                self.bus = Bus::Idle;
                self.status = 0xf8;
                self.twcr &= !TWSTO;
            } else if self.bus == Bus::Started {
                // SLA+W / SLA+R: nobody acknowledges on the empty bus.
                let read = cx.cpu.data[self.c.twdr as usize] & 1 != 0;
                self.act(cx, if read { 0x48 } else { 0x20 }, Bus::Nacked, 9);
            } else if self.bus == Bus::Nacked {
                cx.warn("twi-nack", "TWI: data transfer after a NACKed address (no device on the simulated bus)");
            }
        }
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        if let Some((status, bus)) = self.next.take() {
            self.status = status;
            self.bus = bus;
            self.twcr |= TWINT;
            self.twcr &= !TWSTA;
        }
        self.update_irq(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_DONE);
        *self = Twi::new(self.c.clone());
        cx.cpu.data[self.c.twdr as usize] = 0xff;
        cx.cpu.data[self.c.twar as usize] = 0xfe;
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let scl = cx.sys.clock.hz / self.scl_cycles(cx) as f64;
        vec![
            ("SCL".into(), format!("{scl:.0} Hz")),
            ("Status".into(), format!("0x{:02X}", self.status)),
            ("Bus".into(), "No devices attached (every address is NACKed)".into()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
