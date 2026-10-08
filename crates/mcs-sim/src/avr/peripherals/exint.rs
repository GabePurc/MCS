//! External interrupt INTn (edge/level) and pin-change interrupt (PCINT) controller.

use crate::avr::machine::{Cx, Peripheral, Trigger};

pub struct ExtIntConfig {
    pub eicra: u16,
    pub eimsk: u16,
    pub eifr: u16,
    pub pcicr: u16,
    pub pcifr: u16,
    pub pcmsk: u16,
    pub int0_gpio: u8,
    pub int0_vector: u8,
    pub pc_vector: u8,
    /// GPIO index for PCMSK bit 0..n.
    pub pc_gpios: Vec<u8>,
}

pub struct ExtInt {
    c: ExtIntConfig,
    /// clk_IO halted (power-down/standby/ADC-NR): INT0 edges are not detected.
    io_clock_stopped: bool,
}

impl ExtInt {
    pub fn new(c: ExtIntConfig) -> Self {
        Self { c, io_clock_stopped: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.eicra, 0), (self.c.eimsk, 0), (self.c.eifr, 0xff), (self.c.pcicr, 0), (self.c.pcifr, 0xff)]
    }

    fn update(&self, cx: &mut Cx) {
        let d = &cx.cpu.data;
        let isc = d[self.c.eicra as usize] & 3;
        let int0 = d[self.c.eimsk as usize] & 1 != 0
            && if isc == 0 { cx.sys.pins[self.c.int0_gpio as usize].level == 0 } else { d[self.c.eifr as usize] & 1 != 0 };
        let pc = d[self.c.pcicr as usize] & 1 != 0 && d[self.c.pcifr as usize] & 1 != 0;
        cx.cpu.set_irq(self.c.int0_vector, int0);
        cx.cpu.set_irq(self.c.pc_vector, pc);
    }
}

impl Peripheral for ExtInt {
    fn name(&self) -> &str {
        "EXINT"
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let d = &mut cx.cpu.data;
        let a = addr as usize;
        if addr == self.c.eifr || addr == self.c.pcifr {
            d[a] &= !v; // write one to clear
        } else if addr == self.c.eicra {
            d[a] = v & 3;
        } else {
            d[a] = v & 1;
        }
        self.update(cx);
    }

    fn ack(&mut self, vector: u8, cx: &mut Cx) {
        let d = &mut cx.cpu.data;
        if vector == self.c.int0_vector {
            if d[self.c.eicra as usize] & 3 != 0 {
                d[self.c.eifr as usize] &= !1;
            }
        } else {
            d[self.c.pcifr as usize] &= !1;
        }
        self.update(cx);
    }

    fn on_pin(&mut self, pin: u8, level: u8, cycle: u64, cx: &mut Cx) {
        let a = |r: u16| r as usize;
        if pin == self.c.int0_gpio {
            let isc = cx.cpu.data[a(self.c.eicra)] & 3;
            if isc == 0 {
                self.update(cx);
            } else if !self.io_clock_stopped && (isc == 1 || (isc == 2 && level == 0) || (isc == 3 && level == 1)) {
                cx.cpu.data[a(self.c.eifr)] |= 1;
                cx.sys.trigger(Trigger::Int0, 1, cycle);
                self.update(cx);
            }
        }
        if let Some(bit) = self.c.pc_gpios.iter().position(|&g| g == pin) {
            if cx.cpu.data[a(self.c.pcmsk)] & (1 << bit) != 0 {
                cx.cpu.data[a(self.c.pcifr)] |= 1;
                cx.sys.trigger(Trigger::PcInt, 1, cycle);
                self.update(cx);
            }
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
