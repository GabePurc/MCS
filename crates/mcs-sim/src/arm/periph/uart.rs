//! USART / UART / LPUART (RM0440 section 37-38), asynchronous mode.
//!
//! Frames really travel on the pins: the transmitter drives the TX signal bit by bit (one event
//! per level change) and the receiver samples the RX pin in the middle of each bit after a start
//! bit edge, so the waveform shows the frames and the Serial Monitor (or any stimulus) talks to
//! the pins. Bit time follows BRR, OVER8, PRESC and the APB prescaler (kernel clock = PCLK).
//!
//! Modelled: CR1 (UE, RE, TE, RXNEIE, TCIE, TXEIE, PEIE, PCE, PS, M0/M1, OVER8, FIFOEN), CR2
//! (STOP, MSBFIRST), CR3 (EIE), BRR, PRESC, ISR (PE, FE, ORE, RXNE/RXFNE, TC, TXE/TXFNF, BUSY, TEACK,
//! REACK, TXFE, RXFF), ICR, RQR (RXFRQ, TXFRQ), RDR, TDR and the interrupt line.
//!
//! Simplifications: FIFO mode only deepens the TX/RX queues (8 entries; thresholds and the
//! TXFTIE/RXFTIE interrupts are not modelled); the receiver samples one point per bit instead of
//! the 3-of-16 majority vote (noise flag NE never sets); IDLE and receiver timeout, LIN, smartcard,
//! IrDA, synchronous mode, hardware flow control, DMA and CCIPR clock selection are not modelled;
//! stop bits 0.5 and 1.5 are rounded up to 1 and 2.

use std::collections::VecDeque;

use mcs_core::arm::device::{UartInstance, UartKind};

use crate::arm::bus::{Cx, Mmio};
use crate::arm::sys::sig;

use super::{lane_read, lane_write};

const CR1: u32 = 0x00;
const CR2: u32 = 0x04;
const CR3: u32 = 0x08;
const BRR: u32 = 0x0c;
const GTPR: u32 = 0x10;
const RTOR: u32 = 0x14;
const RQR: u32 = 0x18;
const ISR: u32 = 0x1c;
const ICR: u32 = 0x20;
const RDR: u32 = 0x24;
const TDR: u32 = 0x28;
const PRESC: u32 = 0x2c;

const UE: u32 = 1;
const RE: u32 = 1 << 2;
const TE: u32 = 1 << 3;
const RXNEIE: u32 = 1 << 5;
const TCIE: u32 = 1 << 6;
const TXEIE: u32 = 1 << 7;
const PEIE: u32 = 1 << 8;
const PS: u32 = 1 << 9;
const PCE: u32 = 1 << 10;
const M0: u32 = 1 << 12;
const OVER8: u32 = 1 << 15;
const M1: u32 = 1 << 28;
const FIFOEN: u32 = 1 << 29;
const TXFEIE: u32 = 1 << 30;
const RXFFIE: u32 = 1 << 31;

const F_PE: u32 = 1;
const F_FE: u32 = 1 << 1;
const F_ORE: u32 = 1 << 3;
const F_TC: u32 = 1 << 6;

const EV_TX: u8 = 0;
const EV_RX: u8 = 1;

const FIFO_DEPTH: usize = 8;
const PRESCALERS: [u64; 12] = [1, 2, 4, 6, 8, 10, 12, 16, 32, 64, 128, 256];

pub struct Uart {
    name: String,
    kind: UartKind,
    apb: u8,
    irq: u32,
    en_reg: u8,
    en_bit: u8,
    sig_tx: u16,
    sig_rx: u16,
    cr1: u32,
    cr2: u32,
    cr3: u32,
    brr: u32,
    gtpr: u32,
    rtor: u32,
    presc: u32,
    /// Sticky flags: PE, FE, ORE (+ TC).
    flags: u32,
    // Transmitter
    tx_q: VecDeque<u16>,
    tx_busy: bool,
    tx_frame: [u8; 16],
    tx_len: u8,
    tx_i: u8,
    tx_t0: u64,
    /// Bit time of the frame in progress: `num / den` HCLK cycles.
    tx_num: u64,
    tx_den: u64,
    // Receiver
    rx_q: VecDeque<u16>,
    rx_active: bool,
    rx_i: u8,
    rx_t0: u64,
    rx_bits: u32,
    rx_num: u64,
    rx_den: u64,
}

impl Uart {
    pub fn new(inst: &UartInstance) -> Self {
        let idx = crate::arm::sys::uart_index(&inst.name).max(1);
        Self {
            name: inst.name.clone(),
            kind: inst.kind,
            apb: inst.apb,
            irq: inst.irq as u32,
            en_reg: inst.enable.reg,
            en_bit: inst.enable.bit,
            sig_tx: sig::uart(idx, false),
            sig_rx: sig::uart(idx, true),
            cr1: 0,
            cr2: 0,
            cr3: 0,
            brr: 0,
            gtpr: 0,
            rtor: 0,
            presc: 0,
            flags: 0,
            tx_q: VecDeque::new(),
            tx_busy: false,
            tx_frame: [1; 16],
            tx_len: 0,
            tx_i: 0,
            tx_t0: 0,
            tx_num: 1,
            tx_den: 1,
            rx_q: VecDeque::new(),
            rx_active: false,
            rx_i: 0,
            rx_t0: 0,
            rx_bits: 0,
            rx_num: 1,
            rx_den: 1,
        }
    }

    #[inline]
    fn fifo(&self) -> bool {
        self.cr1 & FIFOEN != 0
    }

    #[inline]
    fn depth(&self) -> usize {
        if self.fifo() { FIFO_DEPTH } else { 1 }
    }

    /// Bits per frame excluding start and stop (7, 8 or 9, parity included).
    fn frame_bits(&self) -> u8 {
        match (self.cr1 & M1 != 0, self.cr1 & M0 != 0) {
            (true, false) => 7,
            (false, true) => 9,
            _ => 8,
        }
    }

    fn data_bits(&self) -> u8 {
        self.frame_bits() - (self.cr1 & PCE != 0) as u8
    }

    fn stop_bits(&self) -> u8 {
        if self.cr2 >> 12 & 2 != 0 { 2 } else { 1 }
    }

    /// Bit time in HCLK cycles as a fraction `(num, den)`.
    fn bit_time(&self, cx: &Cx) -> (u64, u64) {
        let ratio = cx.sys.clk.pclk_div(self.apb) as u64 * PRESCALERS[(self.presc as usize & 0xf).min(11)];
        let (num, den) = if self.kind == UartKind::Lpuart {
            (self.brr as u64 & 0xf_ffff, 256)
        } else if self.cr1 & OVER8 != 0 {
            let div = (self.brr as u64 & 0xfff0) | ((self.brr as u64 & 7) << 1);
            (div, 2)
        } else {
            (self.brr as u64 & 0xffff, 1)
        };
        // At least one HCLK cycle per bit.
        ((num * ratio).max(den), den)
    }

    #[inline]
    fn txe(&self) -> bool {
        self.tx_q.len() < self.depth()
    }

    fn isr(&self) -> u32 {
        let fifo = self.fifo();
        let mut v = self.flags & (F_PE | F_FE | F_ORE | F_TC);
        if self.txe() {
            v |= 1 << 7;
        }
        if !self.rx_q.is_empty() {
            v |= 1 << 5;
        }
        if self.cr1 & UE != 0 {
            if self.cr1 & TE != 0 {
                v |= 1 << 21;
            }
            if self.cr1 & RE != 0 {
                v |= 1 << 22;
            }
        }
        if self.rx_active || self.tx_busy {
            v |= 1 << 16;
        }
        if fifo {
            if self.tx_q.is_empty() {
                v |= 1 << 23;
            }
            if self.rx_q.len() >= FIFO_DEPTH {
                v |= 1 << 24;
            }
        }
        v
    }

    fn update_irq(&self, cx: &mut Cx) {
        let isr = self.isr();
        let c = self.cr1;
        let errors = isr & (F_FE | F_ORE) != 0;
        let level = (c & TXEIE != 0 && isr & 1 << 7 != 0)
            || (c & TCIE != 0 && isr & F_TC != 0)
            || (c & RXNEIE != 0 && isr & (1 << 5 | F_ORE) != 0)
            || (c & PEIE != 0 && isr & F_PE != 0)
            || (self.cr3 & 1 != 0 && errors)
            || (self.fifo() && ((c & TXFEIE != 0 && isr & 1 << 23 != 0) || (c & RXFFIE != 0 && isr & 1 << 24 != 0)));
        cx.set_irq_line(self.irq, level && c & UE != 0);
    }

    // ---- transmitter ----------------------------------------------------------------------

    fn tx_enabled(&self) -> bool {
        self.cr1 & (UE | TE) == (UE | TE)
    }

    /// Starts the next queued frame at `now` if the transmitter is idle.
    fn try_start_tx(&mut self, now: u64, cx: &mut Cx) {
        if self.tx_busy || !self.tx_enabled() {
            return;
        }
        let Some(word) = self.tx_q.pop_front() else {
            return;
        };
        let n = self.frame_bits() as usize;
        let data_n = self.data_bits() as usize;
        let mut f = [1u8; 16];
        f[0] = 0;
        let mut ones = 0u32;
        let msb_first = self.cr2 & (1 << 19) != 0;
        for k in 0..data_n {
            let bit = if msb_first { word >> (data_n - 1 - k) } else { word >> k } & 1;
            ones += bit as u32;
            f[1 + k] = bit as u8;
        }
        if self.cr1 & PCE != 0 {
            let odd = self.cr1 & PS != 0;
            f[1 + data_n] = ((ones & 1) as u8) ^ (odd as u8);
        }
        let stops = self.stop_bits() as usize;
        self.tx_len = (1 + n + stops) as u8;
        self.tx_frame = f;
        for k in 1 + n..1 + n + stops {
            self.tx_frame[k] = 1;
        }
        self.tx_i = 0;
        self.tx_t0 = now;
        (self.tx_num, self.tx_den) = self.bit_time(cx);
        self.tx_busy = true;
        self.flags &= !F_TC;
        cx.sys.sig_out(self.sig_tx, 0, now);
        self.schedule_tx(cx);
    }

    fn schedule_tx(&self, cx: &mut Cx) {
        let k = self.tx_i as u64 + 1;
        let at = self.tx_t0 + k * self.tx_num / self.tx_den;
        cx.schedule(EV_TX, at.max(cx.cycles));
    }

    fn on_tx_event(&mut self, cx: &mut Cx) {
        if !self.tx_busy {
            return;
        }
        let at = cx.cycles;
        self.tx_i += 1;
        if self.tx_i < self.tx_len {
            cx.sys.sig_out(self.sig_tx, self.tx_frame[self.tx_i as usize], at);
            self.schedule_tx(cx);
            return;
        }
        // Frame complete: back-to-back next frame or transmission complete.
        self.tx_busy = false;
        cx.sys.sig_out(self.sig_tx, 1, at);
        if self.tx_q.is_empty() {
            self.flags |= F_TC;
        } else {
            self.try_start_tx(at, cx);
        }
        self.update_irq(cx);
    }

    // ---- receiver -------------------------------------------------------------------------

    fn rx_enabled(&self) -> bool {
        self.cr1 & (UE | RE) == (UE | RE)
    }

    fn rx_level(&self, cx: &Cx) -> u8 {
        cx.sys.pin_of(self.sig_rx).map_or(1, |p| cx.sys.pins[p].level)
    }

    fn sample_time(&self, k: u64) -> u64 {
        self.rx_t0 + (2 * k + 1) * self.rx_num / (2 * self.rx_den)
    }

    fn on_rx_event(&mut self, cx: &mut Cx) {
        if !self.rx_active {
            return;
        }
        let level = self.rx_level(cx);
        let k = self.rx_i as u64;
        let n = self.frame_bits() as u64;
        if k == 0 {
            if level != 0 {
                self.rx_active = false; // glitch, not a start bit
                return;
            }
            self.rx_bits = 0;
        } else if k <= n {
            self.rx_bits |= (level as u32) << (k - 1);
        } else {
            self.finish_rx(level, cx);
            return;
        }
        self.rx_i += 1;
        let at = self.sample_time(self.rx_i as u64).max(cx.cycles + 1);
        cx.schedule(EV_RX, at);
    }

    fn finish_rx(&mut self, stop_level: u8, cx: &mut Cx) {
        self.rx_active = false;
        let n = self.frame_bits() as u32;
        let mut word = self.rx_bits & ((1u32 << n) - 1);
        if self.cr2 & (1 << 19) != 0 {
            word = word.reverse_bits() >> (32 - n);
        }
        if self.cr1 & PCE != 0 {
            let odd = self.cr1 & PS != 0;
            if (word.count_ones() & 1 != 0) != odd {
                self.flags |= F_PE;
            }
            word &= (1 << (n - 1)) - 1;
        }
        if stop_level == 0 {
            self.flags |= F_FE;
        }
        if self.rx_q.len() >= self.depth() {
            self.flags |= F_ORE; // overrun: the new frame is lost
        } else {
            self.rx_q.push_back(word as u16);
        }
        self.update_irq(cx);
    }

    // ---- control ----------------------------------------------------------------------------

    /// UE = 0 resets the transmitter / receiver state machines and the flags.
    fn disable(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TX);
        cx.cancel(EV_RX);
        self.tx_q.clear();
        self.rx_q.clear();
        self.tx_busy = false;
        self.rx_active = false;
        self.flags = F_TC;
        let c = cx.cycles;
        cx.sys.sig_out(self.sig_tx, 1, c);
        self.update_irq(cx);
    }

    fn write_cr1(&mut self, v: u32, cx: &mut Cx) {
        let old = self.cr1;
        self.cr1 = v;
        if old & UE != 0 && v & UE == 0 {
            self.disable(cx);
            return;
        }
        if v & UE != 0 {
            if old & TE == 0 && v & TE != 0 || old & UE == 0 {
                let c = cx.cycles;
                self.try_start_tx(c, cx);
            }
            if v & RE == 0 {
                self.rx_active = false;
                cx.cancel(EV_RX);
            }
        }
        self.update_irq(cx);
    }
}

impl Mmio for Uart {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        if !cx.sys.clock_on(self.en_reg, self.en_bit) {
            return 0;
        }
        if offset & !3 == RDR {
            let w = self.rx_q.pop_front().unwrap_or(0);
            self.update_irq(cx);
            return lane_read(w as u32, offset, size);
        }
        lane_read(self.peek(offset, cx), offset, size)
    }

    fn peek(&mut self, offset: u32, _cx: &mut Cx) -> u32 {
        match offset & !3 {
            CR1 => self.cr1,
            CR2 => self.cr2,
            CR3 => self.cr3,
            BRR => self.brr,
            GTPR => self.gtpr,
            RTOR => self.rtor,
            ISR => self.isr(),
            RDR => self.rx_q.front().copied().unwrap_or(0) as u32,
            TDR => self.tx_q.back().copied().unwrap_or(0) as u32,
            PRESC => self.presc,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if !cx.sys.clock_on(self.en_reg, self.en_bit) {
            return;
        }
        let off = offset & !3;
        match off {
            CR1 => {
                let v = lane_write(self.cr1, offset, size, value);
                self.write_cr1(v, cx);
            }
            CR2 => self.cr2 = lane_write(self.cr2, offset, size, value),
            CR3 => {
                self.cr3 = lane_write(self.cr3, offset, size, value);
                self.update_irq(cx);
            }
            BRR => self.brr = lane_write(self.brr, offset, size, value) & 0xf_ffff,
            GTPR => self.gtpr = lane_write(self.gtpr, offset, size, value),
            RTOR => self.rtor = lane_write(self.rtor, offset, size, value),
            PRESC => self.presc = lane_write(self.presc, offset, size, value) & 0xf,
            RQR => {
                let v = lane_write(0, offset, size, value);
                if v & (1 << 3) != 0 {
                    self.rx_q.clear(); // RXFRQ
                }
                if v & (1 << 4) != 0 {
                    self.tx_q.clear(); // TXFRQ
                }
                self.update_irq(cx);
            }
            ICR => {
                let v = lane_write(0, offset, size, value);
                self.flags &= !(v & (F_PE | F_FE | F_ORE | F_TC));
                self.update_irq(cx);
            }
            TDR if self.tx_q.len() < self.depth() => {
                let mask = if self.frame_bits() == 9 && self.cr1 & PCE == 0 { 0x1ff } else { 0xff };
                self.tx_q.push_back((lane_write(0, offset, size, value) & mask) as u16);
                self.flags &= !F_TC;
                let c = cx.cycles;
                self.try_start_tx(c, cx);
                self.update_irq(cx);
            }
            _ => {}
        }
    }

    fn on_event(&mut self, tag: u8, cx: &mut Cx) {
        if tag == EV_TX {
            self.on_tx_event(cx);
        } else {
            self.on_rx_event(cx);
        }
    }

    fn on_pin(&mut self, pin: usize, level: u8, cycle: u64, cx: &mut Cx) {
        if level != 0 || self.rx_active || !self.rx_enabled() || cx.sys.pin_sig[pin] != self.sig_rx {
            return;
        }
        self.rx_active = true;
        self.rx_i = 0;
        self.rx_t0 = cycle;
        (self.rx_num, self.rx_den) = self.bit_time(cx);
        let at = self.sample_time(0).max(cx.cycles + 1);
        cx.schedule(EV_RX, at);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TX);
        cx.cancel(EV_RX);
        self.cr1 = 0;
        self.cr2 = 0;
        self.cr3 = 0;
        self.brr = 0;
        self.gtpr = 0;
        self.rtor = 0;
        self.presc = 0;
        self.flags = F_TC;
        self.tx_q.clear();
        self.rx_q.clear();
        self.tx_busy = false;
        self.rx_active = false;
        self.update_irq(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        if self.cr1 & UE == 0 {
            return vec![("State".into(), "disabled".into())];
        }
        let (num, den) = self.bit_time(cx);
        let baud = cx.sys.clk.hclk_hz * den as f64 / num as f64;
        let parity = if self.cr1 & PCE == 0 { "N" } else if self.cr1 & PS == 0 { "E" } else { "O" };
        vec![
            ("Baud".into(), format!("{baud:.0} (BRR = {})", self.brr)),
            ("Frame".into(), format!("{}{}{}", self.data_bits(), parity, self.stop_bits())),
            ("TX".into(), format!("{}{}", if self.cr1 & TE != 0 { "enabled" } else { "off" }, if self.tx_busy { ", sending" } else { "" })),
            ("RX".into(), format!("{}, {} queued", if self.cr1 & RE != 0 { "enabled" } else { "off" }, self.rx_q.len())),
            ("Name".into(), self.name.clone()),
        ]
    }
}
