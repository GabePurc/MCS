//! UART0 / UART1 (ESP32-C3 TRM "UART Controller"), asynchronous mode, 128-byte FIFOs.
//!
//! Frames really travel on the pins: the transmitter drives its GPIO matrix output signal (6 / 9) bit by bit
//! (one event per level change) and the receiver samples its input signal in the middle of each bit after a
//! start-bit edge, so the waveform shows the frames and the Serial Monitor (or any stimulus) talks to the
//! pins. Bit time follows the clock source (`CLK_CONF.SCLK_SEL`: 1 APB_CLK, 2 RC_FAST_CLK, 3 XTAL_CLK),
//! `SCLK_DIV_NUM` and `CLKDIV` (integer + `FRAG`/16): baud = source / (SCLK_DIV_NUM + 1) / (CLKDIV + FRAG/16).
//!
//! Modelled: `FIFO` (write = transmit, read = receive), `STATUS` (FIFO counts, RXD/TXD levels), `CONF0`
//! (`PARITY`, `PARITY_EN`, `BIT_NUM`, `STOP_BIT_NUM`, `LOOPBACK`, `RXFIFO_RST`, `TXFIFO_RST`, `RXD_INV`,
//! `TXD_INV`), `CONF1` (FIFO thresholds, `RX_TOUT_EN`), `MEM_CONF.RX_TOUT_THRHD`, `CLKDIV`, `CLK_CONF`, and the
//! interrupts RXFIFO_FULL, TXFIFO_EMPTY (both level conditions that re-assert while true), PARITY_ERR, FRM_ERR,
//! RXFIFO_OVF, RXFIFO_TOUT and TX_DONE. The interrupt line is interrupt matrix source 21 / 22.
//!
//! Simplifications: one sample per bit (no glitch filter), 1.5 stop bits are rounded up to 2, the receive
//! timeout counts `RX_TOUT_THRHD` frame times (assumption: the TRM unit is one character time), the
//! fractional clock divider `SCLK_DIV_A/B` is ignored, hardware / software flow control, RS485, IrDA,
//! autobaud, AT-command detection and DMA (UHCI) are not modelled. The transmitter and receiver work only
//! while the UART clock is enabled in `SYSTEM_PERIP_CLK_EN0` (bit 2 / 5).

use std::collections::VecDeque;

use mcs_core::riscv::device::{RiscvDeviceSpec, RiscvUartInstance};

use crate::riscv::bus::{Cx, Mmio};

use super::misc::RegFile;

const FIFO: u32 = 0x00;
const INT_RAW: u32 = 0x04;
const INT_ST: u32 = 0x08;
const INT_ENA: u32 = 0x0c;
const INT_CLR: u32 = 0x10;
const CLKDIV: u32 = 0x14;
const STATUS: u32 = 0x1c;
const CONF0: u32 = 0x20;
const CONF1: u32 = 0x24;
const MEM_CONF: u32 = 0x60;
const CLK_CONF: u32 = 0x78;

const I_RXFIFO_FULL: u32 = 1;
const I_TXFIFO_EMPTY: u32 = 1 << 1;
const I_PARITY_ERR: u32 = 1 << 2;
const I_FRM_ERR: u32 = 1 << 3;
const I_RXFIFO_OVF: u32 = 1 << 4;
const I_RXFIFO_TOUT: u32 = 1 << 8;
const I_TX_DONE: u32 = 1 << 14;
const I_MASK: u32 = 0x000f_ffff;

const EV_TX: u8 = 0;
const EV_RX: u8 = 1;
const EV_TOUT: u8 = 2;

const FIFO_DEPTH: usize = 128;

pub struct Uart {
    name: String,
    index: u8,
    source: u8,
    sig_tx: usize,
    sig_rx: usize,
    clk_bit: u8,
    regs: RegFile,
    int_raw: u32,
    int_ena: u32,
    // Transmitter
    tx_q: VecDeque<u8>,
    tx_busy: bool,
    tx_frame: [u8; 16],
    tx_len: u8,
    tx_i: u8,
    tx_byte: u8,
    tx_t0: u64,
    tx_num: u64,
    tx_den: u64,
    tx_level: u8,
    // Receiver
    rx_q: VecDeque<u8>,
    rx_active: bool,
    rx_i: u8,
    rx_t0: u64,
    rx_bits: u32,
    rx_num: u64,
    rx_den: u64,
}

impl Uart {
    pub fn new(spec: &RiscvDeviceSpec, inst: &RiscvUartInstance) -> Self {
        let regs = RegFile::from_spec(spec, &inst.name, inst.base, 0x100);
        let base = spec.peripheral_set.uart_signal_base as usize;
        let i = inst.index as usize;
        Self {
            name: inst.name.clone(),
            index: inst.index,
            source: inst.source,
            sig_tx: base + 3 * i,
            sig_rx: base + 3 * i,
            clk_bit: if i == 0 { 2 } else { 5 },
            regs,
            int_raw: I_TXFIFO_EMPTY,
            int_ena: 0,
            tx_q: VecDeque::new(),
            tx_busy: false,
            tx_frame: [1; 16],
            tx_len: 0,
            tx_i: 0,
            tx_byte: 0,
            tx_t0: 0,
            tx_num: 1,
            tx_den: 1,
            tx_level: 1,
            rx_q: VecDeque::new(),
            rx_active: false,
            rx_i: 0,
            rx_t0: 0,
            rx_bits: 0,
            rx_num: 1,
            rx_den: 1,
        }
    }

    fn conf0(&self) -> u32 {
        self.regs.get(CONF0)
    }

    fn data_bits(&self) -> u8 {
        5 + (self.conf0() >> 2 & 3) as u8
    }

    fn parity_en(&self) -> bool {
        self.conf0() >> 1 & 1 != 0
    }

    fn stop_bits(&self) -> u8 {
        if self.conf0() >> 4 & 3 == 1 {
            1
        } else {
            2
        }
    }

    fn frame_bits(&self) -> u8 {
        1 + self.data_bits() + self.parity_en() as u8 + self.stop_bits()
    }

    /// CPU cycles per bit as `num / den`.
    fn bit_time(&self, cx: &Cx) -> (u64, u64) {
        let clk = &cx.sys.clk;
        let src = match self.regs.get(CLK_CONF) >> 20 & 3 {
            2 => clk.rc_fast,
            3 => clk.xtal,
            _ => clk.apb,
        };
        let sdiv = (self.regs.get(CLK_CONF) >> 12 & 0xff) as u64 + 1;
        let cd = self.regs.get(CLKDIV);
        let d16 = ((cd & 0xfff) as u64 * 16 + (cd >> 20 & 15) as u64).max(1);
        let (n, d) = clk.cpu.cycles_per_tick(src, sdiv * d16);
        (n, d * 16)
    }

    fn clock_on(&self, cx: &Cx) -> bool {
        cx.sys.clock_on(0, self.clk_bit)
    }

    fn tx_inverted(&self) -> bool {
        self.conf0() >> 22 & 1 != 0
    }

    fn level_flags(&mut self) {
        let c1 = self.regs.get(CONF1);
        if self.rx_q.len() as u32 >= (c1 & 0x1ff).max(1) {
            self.int_raw |= I_RXFIFO_FULL;
        }
        if (self.tx_q.len() as u32) < (c1 >> 9 & 0x1ff) {
            self.int_raw |= I_TXFIFO_EMPTY;
        }
    }

    fn update_irq(&mut self, cx: &mut Cx) {
        self.level_flags();
        let on = self.int_raw & self.int_ena & I_MASK != 0;
        cx.irq_source(self.source, on);
    }

    // ---- transmitter ------------------------------------------------------------------------

    fn drive_tx(&mut self, level: u8, at: u64, cx: &mut Cx) {
        self.tx_level = level;
        cx.sys.sig_out(self.sig_tx, level ^ self.tx_inverted() as u8, at);
    }

    fn try_start_tx(&mut self, now: u64, cx: &mut Cx) {
        if self.tx_busy || !self.clock_on(cx) {
            return;
        }
        let Some(byte) = self.tx_q.pop_front() else { return };
        let n = self.data_bits() as usize;
        let mut f = [1u8; 16];
        f[0] = 0;
        let mut ones = 0u32;
        for k in 0..n {
            let bit = byte >> k & 1;
            ones += bit as u32;
            f[1 + k] = bit;
        }
        let mut len = 1 + n;
        if self.parity_en() {
            let odd = self.conf0() & 1 != 0;
            // Even parity: the parity bit makes the number of ones even; odd parity: odd.
            f[len] = if odd { (ones & 1 == 0) as u8 } else { (ones & 1) as u8 };
            len += 1;
        }
        let stops = self.stop_bits() as usize;
        for k in 0..stops {
            f[len + k] = 1;
        }
        self.tx_frame = f;
        self.tx_len = (len + stops) as u8;
        self.tx_i = 0;
        self.tx_byte = byte;
        self.tx_t0 = now;
        (self.tx_num, self.tx_den) = self.bit_time(cx);
        self.tx_busy = true;
        self.drive_tx(0, now, cx);
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
            let l = self.tx_frame[self.tx_i as usize];
            self.drive_tx(l, at, cx);
            self.schedule_tx(cx);
            return;
        }
        // Frame complete: back-to-back next frame or TX_DONE.
        self.tx_busy = false;
        self.drive_tx(1, at, cx);
        if self.conf0() >> 14 & 1 != 0 {
            let b = self.tx_byte;
            self.deliver_rx(b, false, false, cx);
        }
        if self.tx_q.is_empty() {
            self.int_raw |= I_TX_DONE;
        } else {
            self.try_start_tx(at, cx);
        }
        self.update_irq(cx);
    }

    // ---- receiver ---------------------------------------------------------------------------

    fn sample_time(&self, k: u64) -> u64 {
        self.rx_t0 + (2 * k + 1) * self.rx_num / (2 * self.rx_den)
    }

    fn rx_level(&self, cx: &Cx) -> u8 {
        cx.sys.input_level(self.sig_rx) ^ (self.conf0() >> 19 & 1) as u8
    }

    fn on_rx_event(&mut self, cx: &mut Cx) {
        if !self.rx_active {
            return;
        }
        let level = self.rx_level(cx);
        let k = self.rx_i as u64;
        let data = self.data_bits() as u64 + self.parity_en() as u64;
        if k == 0 {
            if level != 0 {
                self.rx_active = false; // glitch, not a start bit
                return;
            }
            self.rx_bits = 0;
        } else if k <= data {
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
        let n = self.data_bits() as u32;
        let word = self.rx_bits & ((1 << n) - 1);
        let mut perr = false;
        if self.parity_en() {
            let pbit = self.rx_bits >> n & 1;
            let odd = self.conf0() & 1 != 0;
            perr = ((word.count_ones() + pbit) & 1 != 0) != odd;
        }
        self.deliver_rx(word as u8, perr, stop_level == 0, cx);
    }

    fn deliver_rx(&mut self, byte: u8, parity_err: bool, frame_err: bool, cx: &mut Cx) {
        if parity_err {
            self.int_raw |= I_PARITY_ERR;
        }
        if frame_err {
            self.int_raw |= I_FRM_ERR;
        }
        if self.rx_q.len() >= FIFO_DEPTH {
            self.int_raw |= I_RXFIFO_OVF;
        } else {
            self.rx_q.push_back(byte);
            // Receive timeout: RX_TOUT_THRHD character times without a new byte.
            if self.regs.get(CONF1) >> 21 & 1 != 0 {
                let thr = (self.regs.get(MEM_CONF) >> 16 & 0x3ff) as u64;
                let (num, den) = self.bit_time(cx);
                let at = cx.cycles + thr.max(1) * self.frame_bits() as u64 * num / den;
                cx.schedule(EV_TOUT, at.max(cx.cycles + 1));
            }
        }
        self.update_irq(cx);
    }

    // ---- registers --------------------------------------------------------------------------

    fn status(&self, cx: &Cx) -> u32 {
        let rxd = (self.rx_level(cx) as u32) << 15;
        (self.rx_q.len() as u32) | rxd | (self.tx_q.len() as u32) << 16 | 0x6000_4000 | (self.tx_level as u32 ^ self.tx_inverted() as u32) << 31
    }

    fn read_reg(&self, off: u32, cx: &Cx) -> u32 {
        match off {
            FIFO => self.rx_q.front().copied().unwrap_or(0) as u32,
            INT_RAW => self.int_raw,
            INT_ST => self.int_raw & self.int_ena,
            INT_ENA => self.int_ena,
            STATUS => self.status(cx),
            _ => self.regs.get(off),
        }
    }
}

impl Mmio for Uart {
    fn read(&mut self, off: u32, _size: u8, cx: &mut Cx) -> u32 {
        match off {
            FIFO => {
                let v = self.rx_q.pop_front().unwrap_or(0) as u32;
                self.update_irq(cx);
                v
            }
            INT_CLR => 0,
            _ => self.read_reg(off, cx),
        }
    }

    fn peek(&mut self, off: u32, cx: &mut Cx) -> u32 {
        if off == INT_CLR {
            0
        } else {
            self.read_reg(off & !3, cx)
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        let now = cx.cycles;
        match off {
            FIFO => {
                if self.tx_q.len() < FIFO_DEPTH {
                    self.tx_q.push_back(v as u8);
                }
                self.try_start_tx(now, cx);
                self.update_irq(cx);
            }
            INT_RAW => {
                // Software can set raw bits (test aid on the real chip).
                self.int_raw |= v & I_MASK;
                self.update_irq(cx);
            }
            INT_ENA => {
                self.int_ena = v & I_MASK;
                self.update_irq(cx);
            }
            INT_CLR => {
                self.int_raw &= !(v & I_MASK);
                self.update_irq(cx);
            }
            INT_ST | STATUS => {}
            CONF0 => {
                self.regs.put(off, v & !(3 << 17));
                if v >> 17 & 1 != 0 {
                    self.rx_q.clear();
                }
                if v >> 18 & 1 != 0 {
                    self.tx_q.clear();
                }
                // The TX line idles at the (inverted) idle level.
                let l = self.tx_level;
                self.drive_tx(l, now, cx);
                self.update_irq(cx);
            }
            CONF1 | MEM_CONF => {
                self.regs.put(off, v);
                self.update_irq(cx);
            }
            CLKDIV | CLK_CONF => {
                self.regs.put(off, v);
                self.try_start_tx(now, cx);
            }
            _ => self.regs.put(off, v),
        }
    }

    fn on_event(&mut self, tag: u8, cx: &mut Cx) {
        match tag {
            EV_TX => self.on_tx_event(cx),
            EV_RX => self.on_rx_event(cx),
            _ => {
                if !self.rx_q.is_empty() {
                    self.int_raw |= I_RXFIFO_TOUT;
                    self.update_irq(cx);
                }
            }
        }
    }

    fn on_pin(&mut self, pin: usize, level: u8, cycle: u64, cx: &mut Cx) {
        if level != 0 || self.rx_active || cx.sys.input_pin(self.sig_rx) != Some(pin) || !self.clock_on(cx) {
            return;
        }
        self.rx_active = true;
        self.rx_i = 0;
        self.rx_t0 = cycle;
        (self.rx_num, self.rx_den) = self.bit_time(cx);
        let at = self.sample_time(0).max(cx.cycles + 1);
        cx.schedule(EV_RX, at);
    }

    fn on_clock_change(&mut self, cx: &mut Cx) {
        // A frame in progress keeps the bit time it started with; a UART whose clock was just enabled
        // starts any queued data.
        let now = cx.cycles;
        self.try_start_tx(now, cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        cx.cancel(EV_TX);
        cx.cancel(EV_RX);
        cx.cancel(EV_TOUT);
        self.regs.reset();
        self.tx_q.clear();
        self.rx_q.clear();
        self.tx_busy = false;
        self.rx_active = false;
        self.int_raw = I_TXFIFO_EMPTY;
        self.int_ena = 0;
        let c = cx.cycles;
        self.drive_tx(1, c, cx);
        self.update_irq(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let (num, den) = self.bit_time(cx);
        let baud = cx.sys.clk.cpu.as_f64() * den as f64 / num as f64;
        let parity = if !self.parity_en() { "N" } else if self.conf0() & 1 == 0 { "E" } else { "O" };
        vec![
            ("Name".into(), format!("{} (index {})", self.name, self.index)),
            ("Baud".into(), format!("{baud:.0}")),
            ("Frame".into(), format!("{}{}{}", self.data_bits(), parity, self.stop_bits())),
            ("TX FIFO".into(), format!("{}{}", self.tx_q.len(), if self.tx_busy { ", sending" } else { "" })),
            ("RX FIFO".into(), self.rx_q.len().to_string()),
            ("Clock".into(), if self.clock_on(cx) { "enabled".into() } else { "gated".into() }),
        ]
    }
}
