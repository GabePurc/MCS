//! Analog comparator and 8-bit successive-approximation ADC.

use crate::avr::machine::{Cx, Peripheral, Trigger};

pub struct AcConfig {
    pub acsr: u16,
    pub ain0_gpio: u8,
    pub ain1_gpio: u8,
    pub vector: u8,
}

const ACD: u8 = 0x80;
const ACO: u8 = 0x20;
const ACI: u8 = 0x10;
const ACIE: u8 = 0x08;
const ACIC: u8 = 0x04;

pub struct AnalogComparator {
    c: AcConfig,
    acsr: u8,
    aco: u8,
}

impl AnalogComparator {
    pub fn new(c: AcConfig) -> Self {
        Self { c, acsr: 0, aco: 0 }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.acsr, ACI)]
    }

    /// Whether ACIC routes the comparator output to the timer's input capture.
    pub fn capture_enabled(&self) -> bool {
        self.acsr & ACIC != 0
    }

    fn evaluate(&mut self, cx: &mut Cx) {
        let pins = &cx.sys.pins;
        let out = if self.acsr & ACD != 0 {
            0
        } else {
            (pins[self.c.ain0_gpio as usize].volts > pins[self.c.ain1_gpio as usize].volts) as u8
        };
        if out == self.aco {
            return;
        }
        self.aco = out;
        let now = cx.now();
        let mode = self.acsr & 3;
        if mode == 0 || (mode == 2 && out == 0) || (mode == 3 && out == 1) {
            self.acsr |= ACI;
            cx.sys.trigger(Trigger::Ac, 1, now);
            self.update_irq(cx);
        }
        cx.sys.trigger(Trigger::AcOutput, out, now);
    }

    fn update_irq(&self, cx: &mut Cx) {
        cx.cpu.set_irq(self.c.vector, self.acsr & (ACI | ACIE) == ACI | ACIE);
    }

    fn value(&self) -> u8 {
        (self.acsr & !ACO) | if self.aco != 0 { ACO } else { 0 }
    }
}

impl Peripheral for AnalogComparator {
    fn name(&self) -> &str {
        "AC"
    }

    fn read(&mut self, _addr: u16, _cx: &mut Cx) -> u8 {
        self.value()
    }

    fn peek(&mut self, _addr: u16, _cx: &mut Cx) -> u8 {
        self.value()
    }

    fn write(&mut self, _addr: u16, v: u8, cx: &mut Cx) {
        let aci = self.acsr & ACI & !v; // write 1 clears ACI
        self.acsr = (v & !(ACO | ACI)) | aci;
        self.evaluate(cx);
        self.update_irq(cx);
    }

    fn ack(&mut self, _vector: u8, cx: &mut Cx) {
        self.acsr &= !ACI;
        self.update_irq(cx);
    }

    fn on_analog(&mut self, pin: u8, cx: &mut Cx) {
        if pin == self.c.ain0_gpio || pin == self.c.ain1_gpio {
            self.evaluate(cx);
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.acsr = 0;
        self.aco = 0;
        self.evaluate(cx);
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let p = &cx.sys.pins;
        vec![
            ("AIN0 (V)".into(), format!("{:.3}", p[self.c.ain0_gpio as usize].volts)),
            ("AIN1 (V)".into(), format!("{:.3}", p[self.c.ain1_gpio as usize].volts)),
            ("ACO".into(), self.aco.to_string()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub struct AdcConfig {
    pub adcsra: u16,
    pub adcsrb: u16,
    pub admux: u16,
    pub adcl: u16,
    /// GPIO index per MUX channel.
    pub channels: Vec<usize>,
    pub vector: u8,
    pub prr_mask: u8,
}

const ADEN: u8 = 0x80;
const ADSC: u8 = 0x40;
const ADATE: u8 = 0x20;
const ADIF: u8 = 0x10;
const ADIE: u8 = 0x08;
const ADPS_DIV: [u64; 8] = [2, 2, 4, 8, 16, 32, 64, 128];
const ADTS_TRIGGER: [Option<Trigger>; 8] = [
    None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::Tc0CompA), Some(Trigger::Tc0Ovf), Some(Trigger::Tc0CompB), Some(Trigger::PcInt), Some(Trigger::Tc0Capt),
];
const EV_DONE: u8 = 0;

pub struct Adc {
    c: AdcConfig,
    busy: bool,
    first: bool,
    sample: f64,
    power_reduced: bool,
    clock_stopped: bool,
}

impl Adc {
    pub fn new(c: AdcConfig) -> Self {
        Self { c, busy: false, first: true, sample: 0.0, power_reduced: false, clock_stopped: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        vec![(self.c.adcsra, ADIF), (self.c.adcsrb, 0), (self.c.admux, 0), (self.c.adcl, 0)]
    }

    fn sra(&self, cx: &Cx) -> u8 {
        cx.cpu.data[self.c.adcsra as usize]
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.power_reduced || self.clock_stopped {
            return;
        }
        let div = ADPS_DIV[(self.sra(cx) & 7) as usize];
        let clocks = if self.first { 25 } else { 13 };
        self.first = false;
        self.busy = true;
        let ch = self.c.channels[(cx.cpu.data[self.c.admux as usize] & 3) as usize];
        self.sample = cx.sys.pins[ch].volts;
        let at = cx.now() + clocks * div;
        cx.schedule(EV_DONE, at);
    }

    fn abort(&mut self, cx: &mut Cx) {
        self.busy = false;
        cx.cancel(EV_DONE);
    }

    fn update_irq(&self, cx: &mut Cx) {
        let sra = self.sra(cx);
        cx.cpu.set_irq(self.c.vector, sra & (ADIF | ADIE) == ADIF | ADIE);
    }
}

impl Peripheral for Adc {
    fn name(&self) -> &str {
        "ADC"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let v = cx.cpu.data[addr as usize];
        if addr == self.c.adcsra && self.busy { v | ADSC } else { v }
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        self.read(addr, cx)
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let a = addr as usize;
        if addr == self.c.adcl {
            return; // read-only result
        }
        if addr == self.c.admux {
            cx.cpu.data[a] = v & 0x03;
            return;
        }
        if addr == self.c.adcsrb {
            cx.cpu.data[a] = v & 0x07;
            return;
        }
        let old = cx.cpu.data[a];
        let nv = (v & !(ADSC | ADIF)) | (old & ADIF & !v);
        cx.cpu.data[a] = nv;
        if nv & ADEN == 0 {
            if self.busy {
                self.abort(cx);
            }
            self.first = true;
        } else if v & ADSC != 0 && !self.busy {
            self.start(cx);
        }
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        let vref = cx.sys.vcc;
        let result = ((self.sample * 256.0) / vref).floor().clamp(0.0, 255.0) as u8;
        cx.cpu.data[self.c.adcl as usize] = result;
        cx.cpu.data[self.c.adcsra as usize] |= ADIF;
        self.busy = false;
        let sra = self.sra(cx);
        let free_running = sra & ADATE != 0 && cx.cpu.data[self.c.adcsrb as usize] & 7 == 0 && sra & ADEN != 0;
        if free_running {
            self.start(cx);
        }
        self.update_irq(cx);
    }

    fn ack(&mut self, _vector: u8, cx: &mut Cx) {
        cx.cpu.data[self.c.adcsra as usize] &= !ADIF;
        self.update_irq(cx);
    }

    fn on_trigger(&mut self, trigger: Trigger, _value: u8, _cycle: u64, cx: &mut Cx) {
        let sra = self.sra(cx);
        if sra & (ADEN | ADATE) != ADEN | ADATE || self.busy {
            return;
        }
        let ts = (cx.cpu.data[self.c.adcsrb as usize] & 7) as usize;
        if ts != 0 && ADTS_TRIGGER[ts] == Some(trigger) {
            self.start(cx);
        }
    }

    fn on_power_reduction(&mut self, prr: u8, cx: &mut Cx) {
        self.power_reduced = prr & self.c.prr_mask != 0;
        if self.power_reduced && self.busy {
            self.abort(cx);
        }
    }

    fn on_sleep(&mut self, mode: u8, cx: &mut Cx) {
        if mode == 1 {
            // ADC Noise Reduction: entering sleep starts a conversion.
            if self.sra(cx) & ADEN != 0 && !self.busy {
                self.start(cx);
            }
        } else if mode != 0 {
            self.clock_stopped = true;
            if self.busy {
                self.abort(cx);
            }
        }
    }

    fn on_wake(&mut self, _cx: &mut Cx) {
        self.clock_stopped = false;
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.abort(cx);
        self.first = true;
        self.power_reduced = false;
        self.clock_stopped = false;
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let mux = (cx.cpu.data[self.c.admux as usize] & 3) as usize;
        let ch = self.c.channels[mux];
        let sra = self.sra(cx);
        vec![
            ("State".into(), if self.busy { "Converting" } else if sra & ADEN != 0 { "Idle" } else { "Disabled" }.into()),
            ("Channel".into(), format!("ADC{mux} ({})", cx.sys.pins[ch].name)),
            ("Input (V)".into(), format!("{:.3}", cx.sys.pins[ch].volts)),
            ("Vref (V)".into(), format!("{:.2}", cx.sys.vcc)),
            ("ADC clock div".into(), ADPS_DIV[(sra & 7) as usize].to_string()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
