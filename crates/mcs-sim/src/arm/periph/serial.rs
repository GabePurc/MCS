//! Test-bench serial port (outside the MCU), the simulator side of the Serial Monitor. Same
//! behaviour as the AVR `SerialBridge`:
//! * monitor: decodes UART frames from any GPIO (a USART TX pin), sampling the middle of every bit
//!   after a start bit;
//! * inject: sends bytes typed by the user as UART frames into any GPIO (a USART RX pin), idling high.
//!
//! Timing is in seconds (the PC end of a serial cable does not follow the MCU clock), so a wrong
//! baud rate in the firmware shows up as garbage, as it would on real hardware.

use std::collections::VecDeque;

use crate::arm::bus::Cx;
use crate::avr::peripherals::serial::SerialConfig;
use crate::pins::ExtDrive;

const EV_SAMPLE: u8 = 0;
const EV_SEND: u8 = 1;

pub struct SerialBridge {
    cfg: SerialConfig,
    /// Monitor: bit being sampled (0 = start bit) and the start time.
    rx_bit: Option<u8>,
    rx_t0: f64,
    rx_bits: u16,
    /// Injection: frame bits being sent and their start time.
    tx_queue: VecDeque<u8>,
    tx_frame: Vec<u8>,
    tx_i: usize,
    tx_t0: f64,
}

impl Default for SerialBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl SerialBridge {
    pub fn new() -> Self {
        Self { cfg: SerialConfig::default(), rx_bit: None, rx_t0: 0.0, rx_bits: 0, tx_queue: VecDeque::new(), tx_frame: Vec::new(), tx_i: 0, tx_t0: 0.0 }
    }

    pub fn config(&self) -> SerialConfig {
        self.cfg
    }

    fn bit_s(&self) -> f64 {
        1.0 / self.cfg.baud.clamp(10.0, 10e6)
    }

    fn pin_ok(cx: &Cx, g: usize) -> bool {
        g < cx.sys.pins.len()
    }

    pub fn configure(&mut self, cfg: SerialConfig, cx: &mut Cx) {
        let old_inject = self.cfg.inject;
        let n = cx.sys.pins.len();
        self.cfg = SerialConfig {
            data_bits: cfg.data_bits.clamp(5, 9),
            stop_bits: cfg.stop_bits.clamp(1, 2),
            monitor: cfg.monitor.filter(|&g| g < n),
            inject: cfg.inject.filter(|&g| g < n),
            ..cfg
        };
        self.rx_bit = None;
        cx.cancel(EV_SAMPLE);
        if old_inject != self.cfg.inject {
            self.tx_frame.clear();
            self.tx_queue.clear();
            cx.cancel(EV_SEND);
            let now = cx.now();
            if let Some(g) = old_inject.filter(|&g| Self::pin_ok(cx, g)) {
                cx.sys.pins[g].ext = ExtDrive::Float;
                cx.sys.update_pin(g, now);
            }
            if let Some(g) = self.cfg.inject {
                // An idle UART line is high.
                cx.sys.pins[g].gen = None;
                cx.sys.pins[g].ext = ExtDrive::High;
                cx.sys.update_pin(g, now);
            }
        }
    }

    pub fn send(&mut self, bytes: &[u8], cx: &mut Cx) {
        if self.cfg.inject.is_none() {
            return;
        }
        self.tx_queue.extend(bytes);
        if self.tx_frame.is_empty() {
            self.next_frame(cx);
        }
    }

    fn next_frame(&mut self, cx: &mut Cx) {
        let Some(b) = self.tx_queue.pop_front() else {
            self.tx_frame.clear();
            return;
        };
        let c = self.cfg;
        let mut f = vec![0u8];
        let mut ones = 0;
        for i in 0..c.data_bits {
            let bit = ((b as u16 >> i) & 1) as u8;
            ones += bit;
            f.push(bit);
        }
        if c.parity != 0 {
            f.push(if c.parity == 1 { ones & 1 } else { (ones & 1) ^ 1 });
        }
        f.extend(std::iter::repeat_n(1, c.stop_bits as usize));
        self.tx_frame = f;
        self.tx_i = 0;
        self.tx_t0 = cx.time_seconds();
        self.drive_bit(cx);
    }

    fn drive_bit(&mut self, cx: &mut Cx) {
        let Some(g) = self.cfg.inject else { return };
        let level = self.tx_frame[self.tx_i];
        let now = cx.now();
        let ext = if level != 0 { ExtDrive::High } else { ExtDrive::Low };
        if cx.sys.pins[g].ext != ext {
            cx.sys.pins[g].ext = ext;
            cx.sys.update_pin(g, now);
        }
        let at = cx.sys.clock.cycle_at(self.tx_t0 + (self.tx_i + 1) as f64 * self.bit_s()).max(now + 1);
        cx.schedule(EV_SEND, at);
    }

    fn sample(&mut self, cx: &mut Cx) {
        let (Some(i), Some(g)) = (self.rx_bit, self.cfg.monitor) else { return };
        let level = cx.sys.pins[g].level;
        let c = self.cfg;
        let parity_at = 1 + c.data_bits;
        let stop_at = parity_at + (c.parity != 0) as u8;
        if i == 0 && level != 0 {
            self.rx_bit = None; // glitch, not a start bit
            return;
        }
        if i >= 1 && i < parity_at {
            self.rx_bits |= (level as u16) << (i - 1);
        }
        if i >= stop_at {
            cx.sys.serial_out.push(self.rx_bits as u8);
            if level == 0 {
                let now = cx.now();
                cx.sys.warn_key(now, "serial-framing", "Serial Monitor: framing error (stop bit low) - check the baud rate and frame format");
            }
            self.rx_bit = None;
            return;
        }
        self.rx_bit = Some(i + 1);
        let at = cx.sys.clock.cycle_at(self.rx_t0 + (i as f64 + 1.5) * self.bit_s()).max(cx.now() + 1);
        cx.schedule(EV_SAMPLE, at);
    }

    /// A GPIO changed level (start-bit detection on the monitored pin).
    pub fn on_pin(&mut self, pin: usize, level: u8, cycle: u64, cx: &mut Cx) {
        if Some(pin) != self.cfg.monitor || level != 0 || self.rx_bit.is_some() {
            return;
        }
        self.rx_bit = Some(0);
        self.rx_bits = 0;
        self.rx_t0 = cx.sys.clock.time_at(cycle);
        let at = cx.sys.clock.cycle_at(self.rx_t0 + 0.5 * self.bit_s()).max(cx.now() + 1);
        cx.schedule(EV_SAMPLE, at);
    }

    pub fn on_event(&mut self, tag: u8, cx: &mut Cx) {
        if tag == EV_SAMPLE {
            self.sample(cx);
        } else if !self.tx_frame.is_empty() {
            self.tx_i += 1;
            if self.tx_i < self.tx_frame.len() {
                self.drive_bit(cx);
            } else {
                self.next_frame(cx);
            }
        }
    }

    /// Re-times pending edges on the new cycle grid.
    pub fn on_clock_change(&mut self, cx: &mut Cx) {
        if let Some(i) = self.rx_bit {
            let at = cx.sys.clock.cycle_at(self.rx_t0 + (i as f64 + 0.5) * self.bit_s()).max(cx.now() + 1);
            cx.schedule(EV_SAMPLE, at);
        }
        if !self.tx_frame.is_empty() {
            let at = cx.sys.clock.cycle_at(self.tx_t0 + (self.tx_i + 1) as f64 * self.bit_s()).max(cx.now() + 1);
            cx.schedule(EV_SEND, at);
        }
    }

    /// External equipment: the line settings survive MCU resets; pending frames restart.
    pub fn reset(&mut self, cx: &mut Cx) {
        self.rx_bit = None;
        if let Some(g) = self.cfg.inject.filter(|&g| Self::pin_ok(cx, g)) {
            let now = cx.now();
            if self.tx_frame.is_empty() && cx.sys.pins[g].ext != ExtDrive::High {
                cx.sys.pins[g].ext = ExtDrive::High;
                cx.sys.update_pin(g, now);
            }
        }
        if !self.tx_frame.is_empty() {
            self.tx_t0 = cx.time_seconds();
            self.tx_i = 0;
            self.drive_bit(cx);
        }
    }
}
