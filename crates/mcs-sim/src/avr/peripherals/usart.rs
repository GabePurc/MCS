//! USART (asynchronous mode): baud generator, 5-9 data bits, parity, 1-2 stop bits, double
//! speed, transmit buffer + shift register, two-level receive FIFO with frame/overrun/parity
//! errors, RX complete / data register empty / TX complete interrupts.
//!
//! Frames really travel on the pins: the transmitter drives TXD bit by bit (event per bit) and
//! the receiver samples RXD in the middle of each bit after detecting a start bit, so the
//! waveform shows the frames and the Serial Monitor (or any stimulus) talks to the pins.
//!
//! Source: DS40002061B section 20 (USART0); Atmel-2466T section "Accessing UBRRH/UCSRC" for the
//! URSEL shared register of the ATmega8/16/32. Synchronous and master SPI modes are not modelled.

use std::collections::VecDeque;

use crate::avr::machine::{Cx, Peripheral};

#[derive(Clone)]
pub struct UsartConfig {
    pub name: &'static str,
    pub udr: u16,
    pub ucsra: u16,
    pub ucsrb: u16,
    pub ucsrc: u16,
    pub ubrrl: u16,
    pub ubrrh: u16,
    /// UBRRH and UCSRC share one I/O address (`ucsrc == ubrrh`) selected by URSEL (bit 7) on
    /// writes and by a back-to-back read sequence on reads (ATmega8/16/32).
    pub ursel: bool,
    pub rx_gpio: usize,
    pub tx_gpio: usize,
    pub v_rx: u8,
    pub v_udre: u8,
    pub v_tx: u8,
    pub prr_mask: u8,
}

// UCSRnA
const RXC: u8 = 0x80;
const TXC: u8 = 0x40;
const UDRE: u8 = 0x20;
const FE: u8 = 0x10;
const DOR: u8 = 0x08;
const UPE: u8 = 0x04;
const U2X: u8 = 0x02;
const MPCM: u8 = 0x01;
// UCSRnB
const RXCIE: u8 = 0x80;
const TXCIE: u8 = 0x40;
const UDRIE: u8 = 0x20;
const RXEN: u8 = 0x10;
const TXEN: u8 = 0x08;
const UCSZ2: u8 = 0x04;
const RXB8: u8 = 0x02;
const TXB8: u8 = 0x01;

const EV_TX: u8 = 0;
const EV_RX: u8 = 1;

#[derive(Clone, Copy, Default)]
struct Frame {
    data_bits: u8,
    parity: u8, // 0 none, 2 even, 3 odd (UPM1:0)
    stop_bits: u8,
}

struct TxShift {
    bits: [u8; 13],
    n: u8,
    i: u8,
}

pub struct Usart {
    c: UsartConfig,
    ucsra: u8,
    ucsrb: u8,
    ucsrc: u8,
    ubrr: u16,
    tx_buf: Option<u16>,
    tx: Option<TxShift>,
    /// Receive FIFO: (data incl. 9th bit, FE/DOR/UPE flags).
    rx_fifo: VecDeque<(u16, u8)>,
    /// Receiver: Some(bit index) while a frame is being sampled (0 = start bit).
    rx_bit: Option<u8>,
    rx_bits: u16,
    rx_frame: Frame,
    rx_parity_err: bool,
    overrun: bool,
    /// Cycle of the last read of the shared UBRRH/UCSRC address that returned UBRRH.
    last_ubrrh_read: Option<u64>,
}

impl Usart {
    pub fn new(c: UsartConfig) -> Self {
        Self {
            ucsra: UDRE,
            ucsrb: 0,
            ucsrc: if c.ursel { 0x86 } else { 0x06 },
            ubrr: 0,
            tx_buf: None,
            tx: None,
            rx_fifo: VecDeque::with_capacity(3),
            rx_bit: None,
            rx_bits: 0,
            rx_frame: Frame::default(),
            rx_parity_err: false,
            overrun: false,
            last_ubrrh_read: None,
            c,
        }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let c = &self.c;
        let mut v = vec![(c.udr, 0), (c.ucsra, TXC), (c.ucsrb, 0), (c.ucsrc, 0), (c.ubrrl, 0)];
        if c.ubrrh != c.ucsrc {
            v.push((c.ubrrh, 0));
        }
        v
    }

    pub fn vectors(&self) -> [Option<u8>; 3] {
        [Some(self.c.v_rx), Some(self.c.v_udre), Some(self.c.v_tx)]
    }

    fn frame(&self) -> Frame {
        let sz = ((self.ucsrc >> 1) & 3) | if self.ucsrb & UCSZ2 != 0 { 4 } else { 0 };
        let data_bits = match sz {
            0 => 5,
            1 => 6,
            2 => 7,
            7 => 9,
            _ => 8,
        };
        Frame { data_bits, parity: (self.ucsrc >> 4) & 3, stop_bits: if self.ucsrc & 0x08 != 0 { 2 } else { 1 } }
    }

    /// CPU cycles per bit.
    fn bit_cycles(&self) -> u64 {
        (self.ubrr as u64 + 1) * if self.ucsra & U2X != 0 { 8 } else { 16 }
    }

    fn baud(&self, cx: &Cx) -> f64 {
        cx.sys.clock.hz / self.bit_cycles() as f64
    }

    fn update_irq(&self, cx: &mut Cx) {
        let b = self.ucsrb;
        cx.cpu.set_irq(self.c.v_rx, b & RXCIE != 0 && !self.rx_fifo.is_empty());
        cx.cpu.set_irq(self.c.v_udre, b & UDRIE != 0 && self.tx_buf.is_none());
        cx.cpu.set_irq(self.c.v_tx, b & TXCIE != 0 && self.ucsra & TXC != 0);
    }

    /// TXD/RXD port overrides (DS40002061B table 14-9).
    fn apply_pins(&mut self, level: u8, cx: &mut Cx) {
        let now = cx.now();
        let tx_on = self.ucsrb & TXEN != 0 || self.tx.is_some();
        let p = &mut cx.sys.pins[self.c.tx_gpio];
        let (ddoe, ov) = (tx_on as u8, tx_on as u8);
        if p.ddoe != ddoe || p.ddov != ddoe || p.ov_enable != ov || p.ov_value != level {
            p.ddoe = ddoe;
            p.ddov = ddoe;
            p.ov_enable = ov;
            p.ov_value = level;
            cx.sys.update_pin(self.c.tx_gpio, now);
        }
        let rx_on = (self.ucsrb & RXEN != 0) as u8;
        let p = &mut cx.sys.pins[self.c.rx_gpio];
        if p.ddoe != rx_on || p.ddov != 0 {
            p.ddoe = rx_on;
            p.ddov = 0;
            cx.sys.update_pin(self.c.rx_gpio, now);
        }
    }

    fn start_tx(&mut self, cx: &mut Cx) {
        let Some(data) = self.tx_buf.take() else { return };
        let f = self.frame();
        let mut bits = [1u8; 13];
        let mut n = 0usize;
        bits[n] = 0; // start
        n += 1;
        let mut ones = 0;
        for i in 0..f.data_bits {
            let b = ((data >> i) & 1) as u8;
            ones += b;
            bits[n] = b;
            n += 1;
        }
        if f.parity >= 2 {
            bits[n] = (ones & 1) ^ (f.parity & 1);
            n += 1;
        }
        n += f.stop_bits as usize; // stop bits are 1
        self.tx = Some(TxShift { bits, n: n as u8, i: 0 });
        self.ucsra |= UDRE;
        self.apply_pins(0, cx);
        let at = cx.now() + self.bit_cycles();
        cx.schedule(EV_TX, at);
    }

    fn tx_event(&mut self, cx: &mut Cx) {
        let Some(t) = self.tx.as_mut() else { return };
        t.i += 1;
        if t.i < t.n {
            let level = t.bits[t.i as usize];
            self.apply_pins(level, cx);
            let at = cx.now() + self.bit_cycles();
            cx.schedule(EV_TX, at);
            return;
        }
        self.tx = None;
        if self.tx_buf.is_some() && self.ucsrb & TXEN != 0 {
            self.start_tx(cx);
        } else {
            self.ucsra |= TXC;
            self.apply_pins(1, cx);
        }
    }

    fn rx_event(&mut self, cx: &mut Cx) {
        let Some(i) = self.rx_bit else { return };
        let level = cx.sys.pins[self.c.rx_gpio].level;
        let f = self.rx_frame;
        let parity_at = 1 + f.data_bits;
        let stop_at = parity_at + (f.parity >= 2) as u8;
        if i == 0 {
            if level != 0 {
                self.rx_bit = None; // false start bit
                return;
            }
        } else if i < parity_at {
            self.rx_bits |= (level as u16) << (i - 1);
        } else if i < stop_at {
            let ones = self.rx_bits.count_ones() as u8 + level;
            self.rx_parity_err = ones & 1 != f.parity & 1;
        } else {
            // First stop bit: frame complete.
            let mut flags = if level == 0 { FE } else { 0 };
            if self.rx_parity_err {
                flags |= UPE;
            }
            if self.rx_fifo.len() >= 2 {
                self.overrun = true;
            } else {
                if self.overrun {
                    flags |= DOR;
                    self.overrun = false;
                }
                self.rx_fifo.push_back((self.rx_bits, flags));
            }
            self.rx_bit = None;
            self.update_irq(cx);
            return;
        }
        self.rx_bit = Some(i + 1);
        let at = cx.now() + self.bit_cycles();
        cx.schedule(EV_RX, at);
    }

    fn status(&self) -> u8 {
        let mut a = self.ucsra & (TXC | UDRE | U2X | MPCM);
        if self.tx_buf.is_some() {
            a &= !UDRE;
        }
        if let Some(&(_, flags)) = self.rx_fifo.front() {
            a |= RXC | flags;
        }
        a
    }

    fn ctrl_b(&self) -> u8 {
        let rxb8 = self.rx_fifo.front().is_some_and(|e| e.0 & 0x100 != 0);
        (self.ucsrb & !RXB8) | if rxb8 { RXB8 } else { 0 }
    }
}

impl Peripheral for Usart {
    fn name(&self) -> &str {
        self.c.name
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        if addr == self.c.udr {
            let v = self.rx_fifo.pop_front().map(|e| e.0 as u8).unwrap_or(0);
            self.update_irq(cx);
            return v;
        }
        if self.c.ursel && addr == self.c.ucsrc {
            // First read returns UBRRH; a read in the very next cycle returns UCSRC (URSEL = 1).
            let now = cx.now();
            if self.last_ubrrh_read.is_some_and(|c| c + 1 == now) {
                self.last_ubrrh_read = None;
                return self.ucsrc | 0x80;
            }
            self.last_ubrrh_read = Some(now);
        }
        self.peek(addr, cx)
    }

    fn peek(&mut self, addr: u16, _cx: &mut Cx) -> u8 {
        let c = &self.c;
        if addr == c.udr {
            self.rx_fifo.front().map(|e| e.0 as u8).unwrap_or(0)
        } else if addr == c.ucsra {
            self.status()
        } else if addr == c.ucsrb {
            self.ctrl_b()
        } else if addr == c.ucsrc && !c.ursel {
            self.ucsrc
        } else if addr == c.ubrrl {
            self.ubrr as u8
        } else {
            (self.ubrr >> 8) as u8
        }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let c = &self.c;
        if addr == c.udr {
            if self.tx_buf.is_some() {
                cx.warn("udr-full", "UDR written while the transmit buffer is full (data lost): wait for UDRE");
                return;
            }
            let ninth = if self.ucsrb & TXB8 != 0 { 0x100 } else { 0 };
            self.tx_buf = Some(v as u16 | ninth);
            self.ucsra &= !UDRE;
            if self.tx.is_none() && self.ucsrb & TXEN != 0 {
                self.start_tx(cx);
            }
        } else if addr == c.ucsra {
            if v & TXC != 0 {
                self.ucsra &= !TXC;
            }
            self.ucsra = (self.ucsra & !(U2X | MPCM)) | (v & (U2X | MPCM));
        } else if addr == c.ucsrb {
            let old = self.ucsrb;
            self.ucsrb = v & !RXB8;
            if old & RXEN != 0 && v & RXEN == 0 {
                // Disabling the receiver flushes the FIFO.
                self.rx_fifo.clear();
                self.rx_bit = None;
                cx.cancel(EV_RX);
            }
            let line = self.tx.as_ref().map(|t| t.bits[t.i as usize]).unwrap_or(1);
            self.apply_pins(line, cx);
            if self.tx.is_none() && self.tx_buf.is_some() && v & TXEN != 0 {
                self.start_tx(cx);
            }
        } else if addr == c.ucsrc && (!c.ursel || v & 0x80 != 0) {
            if v & if c.ursel { 0x40 } else { 0xc0 } != 0 {
                cx.warn("usart-sync", "USART synchronous / master SPI modes are not simulated (UMSEL != 0)");
            }
            self.ucsrc = v;
        } else if addr == c.ubrrl {
            self.ubrr = (self.ubrr & 0x0f00) | v as u16;
        } else {
            self.ubrr = (self.ubrr & 0x00ff) | (((v & 0x0f) as u16) << 8);
        }
        self.update_irq(cx);
    }

    fn on_event(&mut self, tag: u8, _cycle: u64, cx: &mut Cx) {
        if tag == EV_TX {
            self.tx_event(cx);
        } else {
            self.rx_event(cx);
        }
        self.update_irq(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {
        if pin as usize != self.c.rx_gpio || level != 0 || self.rx_bit.is_some() || self.ucsrb & RXEN == 0 {
            return;
        }
        // Start bit edge: sample its middle, then every bit.
        self.rx_bit = Some(0);
        self.rx_bits = 0;
        self.rx_parity_err = false;
        self.rx_frame = self.frame();
        let at = cycle + (self.bit_cycles() / 2).max(1);
        cx.schedule(EV_RX, at.max(cx.now()));
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        if vector == self.c.v_tx {
            self.ucsra &= !TXC;
        }
        self.update_irq(cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TX);
        cx.cancel(EV_RX);
        *self = Usart::new(self.c.clone());
        self.apply_pins(1, cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let f = self.frame();
        let parity = match f.parity {
            2 => "E",
            3 => "O",
            _ => "N",
        };
        let state = match (self.ucsrb & TXEN != 0, self.ucsrb & RXEN != 0) {
            (true, true) => "TX + RX enabled",
            (true, false) => "TX enabled",
            (false, true) => "RX enabled",
            _ => "Disabled",
        };
        vec![
            ("State".into(), state.into()),
            ("Baud rate".into(), format!("{:.0}", self.baud(cx))),
            ("Frame".into(), format!("{}{}{}", f.data_bits, parity, f.stop_bits)),
            ("TX".into(), if self.tx.is_some() { "Sending" } else { "Idle" }.into()),
            ("RX FIFO".into(), format!("{} byte(s)", self.rx_fifo.len())),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
