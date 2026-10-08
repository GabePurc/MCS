//! External interrupts INTn (level / edge) and pin-change interrupt groups (PCINTn).
//! Register layouts differ per family (EICRA/EIMSK/EIFR + PCICR/PCIFR/PCMSKn on the ATtiny10 and
//! the ATmegas; MCUCR.ISC0 + GIMSK/GIFR + PCMSK on the ATtiny85), so every bit is configured.
//! Sources: Atmel-8127H section 9, DS40002061B section 13, Atmel-2586Q section 9.

use crate::avr::machine::{Cx, Peripheral, Trigger};

/// One INTn input.
#[derive(Clone)]
pub struct IntSpec {
    pub gpio: u8,
    pub vector: u8,
    /// Interrupt sense control field: register and the field's lowest bit position.
    pub isc_reg: u16,
    pub isc_shift: u8,
    pub mask_reg: u16,
    pub mask_bit: u8,
    pub flag_reg: u16,
    pub flag_bit: u8,
}

/// One pin-change group (PCINTn vector).
#[derive(Clone)]
pub struct PcGroupSpec {
    /// GPIO index per PCMSK bit.
    pub gpios: Vec<u8>,
    pub msk_reg: u16,
    pub vector: u8,
    pub enable_reg: u16,
    pub enable_bit: u8,
    pub flag_reg: u16,
    pub flag_bit: u8,
}

pub struct ExtIntConfig {
    pub ints: Vec<IntSpec>,
    pub groups: Vec<PcGroupSpec>,
    /// Registers this peripheral owns: (address, writable mask, write-one-to-clear flag register).
    pub owned: Vec<(u16, u8, bool)>,
}

pub struct ExtInt {
    c: ExtIntConfig,
    /// clk_IO halted (power-down/standby/ADC-NR): INTn edges are not detected.
    io_clock_stopped: bool,
}

impl ExtInt {
    pub fn new(c: ExtIntConfig) -> Self {
        Self { c, io_clock_stopped: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        self.c.owned.iter().map(|&(a, _, w1c)| (a, if w1c { 0xff } else { 0 })).collect()
    }

    pub fn vectors(&self) -> Vec<Option<u8>> {
        self.c.ints.iter().map(|i| Some(i.vector)).chain(self.c.groups.iter().map(|g| Some(g.vector))).collect()
    }

    fn isc(&self, i: &IntSpec, cx: &Cx) -> u8 {
        (cx.cpu.data[i.isc_reg as usize] >> i.isc_shift) & 3
    }

    fn update(&self, cx: &mut Cx) {
        for i in &self.c.ints {
            let d = &cx.cpu.data;
            let enabled = d[i.mask_reg as usize] & i.mask_bit != 0;
            let pending = if self.isc(i, cx) == 0 { cx.sys.pins[i.gpio as usize].level == 0 } else { d[i.flag_reg as usize] & i.flag_bit != 0 };
            cx.cpu.set_irq(i.vector, enabled && pending);
        }
        for g in &self.c.groups {
            let d = &cx.cpu.data;
            let p = d[g.enable_reg as usize] & g.enable_bit != 0 && d[g.flag_reg as usize] & g.flag_bit != 0;
            cx.cpu.set_irq(g.vector, p);
        }
    }
}

impl Peripheral for ExtInt {
    fn name(&self) -> &str {
        "EXINT"
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let Some(&(_, mask, w1c)) = self.c.owned.iter().find(|o| o.0 == addr) else { return };
        let d = &mut cx.cpu.data[addr as usize];
        if w1c {
            *d &= !v;
        } else {
            *d = v & mask;
        }
        self.update(cx);
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        if let Some(i) = self.c.ints.iter().find(|i| i.vector == vector) {
            // Level interrupts have no flag; edge flags clear when the vector executes.
            if self.isc(i, cx) != 0 {
                cx.cpu.data[i.flag_reg as usize] &= !i.flag_bit;
            }
        } else if let Some(g) = self.c.groups.iter().find(|g| g.vector == vector) {
            cx.cpu.data[g.flag_reg as usize] &= !g.flag_bit;
        }
        self.update(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {
        let mut changed = false;
        for (k, i) in self.c.ints.iter().enumerate() {
            if pin != i.gpio {
                continue;
            }
            let isc = self.isc(i, cx);
            if isc == 0 {
                changed = true;
            } else if !self.io_clock_stopped && (isc == 1 || (isc == 2 && level == 0) || (isc == 3 && level == 1)) {
                cx.cpu.data[i.flag_reg as usize] |= i.flag_bit;
                if k == 0 {
                    cx.sys.trigger(Trigger::Int0, 1, cycle);
                }
                changed = true;
            }
        }
        for g in &self.c.groups {
            if let Some(bit) = g.gpios.iter().position(|&x| x == pin) {
                if cx.cpu.data[g.msk_reg as usize] & (1 << bit) != 0 {
                    cx.cpu.data[g.flag_reg as usize] |= g.flag_bit;
                    cx.sys.trigger(Trigger::PcInt, 1, cycle);
                    changed = true;
                }
            }
        }
        if changed {
            self.update(cx);
        }
    }

    fn on_reg_written(&mut self, addr: u16, cx: &mut Cx) {
        // ISC bits living in a register owned elsewhere (ATtiny85 MCUCR).
        if self.c.ints.iter().any(|i| i.isc_reg == addr) {
            self.update(cx);
        }
    }

    fn on_sleep(&mut self, mode: u8, _cx: &mut Cx) {
        self.io_clock_stopped = mode != 0;
    }

    fn on_wake(&mut self, _cx: &mut Cx) {
        self.io_clock_stopped = false;
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.io_clock_stopped = false;
        self.update(cx);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
