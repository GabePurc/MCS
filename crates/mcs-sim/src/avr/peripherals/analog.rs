//! Analog comparator and successive-approximation ADC (8-bit ATtiny10, 10-bit classic AVRs).
//! Sources: Atmel-8127H sections 13-14, DS40002061B sections 23-24, Atmel-2586Q sections 16-17,
//! Atmel-2486AA / 2466T / 2503Q (ATmega8: ADFR free running; ATmega16/32: SFIOR trigger select,
//! always-signed differential channels with 10x / 200x gain).

use crate::avr::machine::{Cx, Peripheral, Trigger};

/// Bandgap reference (typical, datasheet electrical characteristics).
pub const BANDGAP_V: f64 = 1.1;

/// Comparator inputs can come from the ADC multiplexer (ACME).
#[derive(Clone)]
pub struct AcmeConfig {
    /// Register and bit of ACME (ADCSRB.ACME).
    pub reg: u16,
    pub bit: u8,
    pub adcsra: u16,
    pub admux: u16,
    pub mux_mask: u8,
    /// GPIO per multiplexer value (None = not a pin).
    pub channels: Vec<Option<usize>>,
}

pub struct AcConfig {
    pub acsr: u16,
    pub ain0_gpio: u8,
    pub ain1_gpio: u8,
    pub vector: u8,
    /// Bandgap reference voltage (V) used when ACBG selects it as the positive input.
    pub bandgap_v: f64,
    /// ACBG selects the bandgap as the positive input.
    pub acbg: bool,
    /// ACIC routes the output to Timer1 input capture.
    pub acic: bool,
    pub acme: Option<AcmeConfig>,
}

const ACD: u8 = 0x80;
const ACBG: u8 = 0x40;
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

    /// (positive, negative) input voltages and the negative input's name.
    fn inputs(&self, cx: &Cx) -> (f64, f64, String) {
        let pins = &cx.sys.pins;
        let pos = if self.c.acbg && self.acsr & ACBG != 0 { self.c.bandgap_v } else { pins[self.c.ain0_gpio as usize].volts };
        if let Some(m) = &self.c.acme {
            let d = &cx.cpu.data;
            if d[m.reg as usize] & m.bit != 0 && d[m.adcsra as usize] & 0x80 == 0 {
                let mux = (d[m.admux as usize] & m.mux_mask) as usize;
                if let Some(Some(g)) = m.channels.get(mux) {
                    return (pos, pins[*g].volts, pins[*g].name.clone());
                }
            }
        }
        let ain1 = &pins[self.c.ain1_gpio as usize];
        (pos, ain1.volts, ain1.name.clone())
    }

    fn evaluate(&mut self, cx: &mut Cx) {
        let out = if self.acsr & ACD != 0 {
            0
        } else {
            let (p, n, _) = self.inputs(cx);
            (p > n) as u8
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

    fn writable(&self) -> u8 {
        let mut m = ACD | ACIE | 0x03;
        if self.c.acbg {
            m |= ACBG;
        }
        if self.c.acic {
            m |= ACIC;
        }
        m
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
        let old_acic = self.acsr & ACIC;
        self.acsr = (v & self.writable()) | aci;
        if self.acsr & ACIC != old_acic {
            let now = cx.now();
            cx.sys.trigger(Trigger::AcCapture, (self.acsr & ACIC != 0) as u8, now);
        }
        self.evaluate(cx);
        self.update_irq(cx);
    }

    fn ack(&mut self, _vector: u8, cx: &mut Cx) {
        self.acsr &= !ACI;
        self.update_irq(cx);
    }

    fn on_analog(&mut self, _pin: u8, cx: &mut Cx) {
        self.evaluate(cx);
    }

    fn on_reg_written(&mut self, addr: u16, cx: &mut Cx) {
        if self.c.acme.as_ref().is_some_and(|m| addr == m.reg || addr == m.admux || addr == m.adcsra) {
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
        let (p, n, nname) = self.inputs(cx);
        let pname = if self.c.acbg && self.acsr & ACBG != 0 { "Bandgap".to_string() } else { cx.sys.pins[self.c.ain0_gpio as usize].name.clone() };
        vec![
            (format!("+ input {pname} (V)"), format!("{p:.3}")),
            (format!("- input {nname} (V)"), format!("{n:.3}")),
            ("ACO".into(), self.aco.to_string()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// What a multiplexer setting converts.
#[derive(Clone, Copy, Debug)]
pub enum AdcInput {
    Pin(usize),
    /// Differential (positive, negative, gain).
    Diff(usize, usize, f64),
    /// Internal voltage (bandgap, GND).
    Volts(f64),
    /// Temperature sensor (voltage at 25 °C).
    Temp(f64),
}

/// Reference voltage options.
#[derive(Clone, Copy, Debug)]
pub enum AdcRef {
    Vcc,
    Volts(f64),
    /// External AREF (pin GPIO index, or None when AREF is a dedicated pin tied to VCC here).
    Aref(Option<usize>),
}

pub struct AdcConfig {
    pub adcsra: u16,
    /// Register holding the auto trigger source field ADTS (ADCSRB; SFIOR on the ATmega16/32).
    /// None: no trigger select (ATmega8): ADATE/ADFR starts free running mode.
    pub adcsrb: Option<u16>,
    /// Bit position of the ADTS field in `adcsrb` (0 in ADCSRB, 5 in SFIOR).
    pub adts_shift: u8,
    /// False when another model owns `adcsrb` (SFIOR is owned by `Gtccr`): the ADC then neither
    /// claims nor writes it, it only reads the trigger field.
    pub adcsrb_owned: bool,
    pub admux: u16,
    pub adcl: u16,
    /// ADCH (10-bit converters).
    pub adch: Option<u16>,
    pub mux_mask: u8,
    /// Input per multiplexer value (None = reserved).
    pub inputs: Vec<Option<AdcInput>>,
    /// Reference selection: (mask in ADMUX, extra mask, reference per combined value).
    /// The selection value is `(admux & mask) >> mask.trailing_zeros()` plus, when `ref_extra`
    /// is set, 4 for that bit (ATtiny85 REFS2).
    pub ref_mask: u8,
    pub ref_extra: u8,
    pub refs: Vec<Option<AdcRef>>,
    /// ADLAR bit mask (0 = none); in ADMUX, or in ADCSRB when `adlar_srb` (ATtiny24A/44A/84A).
    pub adlar: u8,
    pub adlar_srb: bool,
    /// Writable ADMUX / ADCSRB bits.
    pub admux_mask: u8,
    pub adcsrb_mask: u8,
    /// Bipolar input mode bit in ADCSRB (ATtiny85 BIN).
    pub bin: u8,
    /// Differential conversions are always two's complement (-512..511 at 512 / VREF per unit,
    /// ATmega16/32).
    pub diff_signed: bool,
    /// Auto trigger source per ADTS value.
    pub triggers: [Option<Trigger>; 8],
    pub vector: u8,
    pub prr_mask: u8,
    /// Announce ADMUX/ADCSRB/ADCSRA writes (the comparator reads ACME and the mux).
    pub notify: bool,
}

const ADEN: u8 = 0x80;
const ADSC: u8 = 0x40;
const ADATE: u8 = 0x20;
const ADIF: u8 = 0x10;
const ADIE: u8 = 0x08;
const ADPS_DIV: [u64; 8] = [2, 2, 4, 8, 16, 32, 64, 128];
const EV_DONE: u8 = 0;

/// Trigger table of the ATtiny4/5/9/10 (ADCSRB.ADTS, Atmel-8127H table 14-5).
pub const TINY10_TRIGGERS: [Option<Trigger>; 8] = [
    None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::TimerCompA(0)), Some(Trigger::TimerOvf(0)), Some(Trigger::TimerCompB(0)), Some(Trigger::PcInt), Some(Trigger::TimerCapt(0)),
];

pub struct Adc {
    c: AdcConfig,
    busy: bool,
    first: bool,
    /// Sampled input (V) and the reference used.
    sample: f64,
    vref: f64,
    diff: bool,
    /// ADCL was read: data registers keep their value until ADCH is read.
    locked: bool,
    power_reduced: bool,
    clock_stopped: bool,
}

impl Adc {
    pub fn new(c: AdcConfig) -> Self {
        Self { c, busy: false, first: true, sample: 0.0, vref: 5.0, diff: false, locked: false, power_reduced: false, clock_stopped: false }
    }

    pub fn registers(&self) -> Vec<(u16, u8)> {
        let mut v = vec![(self.c.adcsra, ADIF), (self.c.admux, 0), (self.c.adcl, 0)];
        if let (Some(a), true) = (self.c.adcsrb, self.c.adcsrb_owned) {
            v.push((a, 0));
        }
        v.extend(self.c.adch.map(|a| (a, 0)));
        v
    }

    fn sra(&self, cx: &Cx) -> u8 {
        cx.cpu.data[self.c.adcsra as usize]
    }

    /// ADTS field (0 = free running; also when there is no trigger select register).
    fn trigger_select(&self, cx: &Cx) -> usize {
        self.c.adcsrb.map_or(0, |a| ((cx.cpu.data[a as usize] >> self.c.adts_shift) & 7) as usize)
    }

    fn mux(&self, cx: &Cx) -> usize {
        (cx.cpu.data[self.c.admux as usize] & self.c.mux_mask) as usize
    }

    fn reference(&self, cx: &Cx) -> f64 {
        let admux = cx.cpu.data[self.c.admux as usize];
        let mut sel = if self.c.ref_mask == 0 { 0 } else { ((admux & self.c.ref_mask) >> self.c.ref_mask.trailing_zeros()) as usize };
        if self.c.ref_extra != 0 && admux & self.c.ref_extra != 0 {
            sel += 4;
        }
        match self.c.refs.get(sel).copied().flatten() {
            Some(AdcRef::Volts(v)) => v,
            Some(AdcRef::Aref(Some(g))) => cx.sys.pins[g].volts.max(0.1),
            _ => cx.sys.vcc,
        }
    }

    /// Input voltage of the current channel (difference x gain for differential channels).
    fn input(&self, cx: &Cx) -> (f64, bool) {
        let pins = &cx.sys.pins;
        match self.c.inputs.get(self.mux(cx)).copied().flatten() {
            Some(AdcInput::Pin(g)) => (pins[g].volts, false),
            Some(AdcInput::Diff(p, n, gain)) => ((pins[p].volts - pins[n].volts) * gain, true),
            Some(AdcInput::Volts(v)) | Some(AdcInput::Temp(v)) => (v, false),
            None => (0.0, false),
        }
    }

    fn start(&mut self, cx: &mut Cx) {
        if self.power_reduced || self.clock_stopped {
            return;
        }
        let div = ADPS_DIV[(self.sra(cx) & 7) as usize];
        let clocks = if self.first { 25 } else { 13 };
        self.first = false;
        self.busy = true;
        let (v, diff) = self.input(cx);
        self.sample = v;
        self.diff = diff;
        self.vref = self.reference(cx);
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

    /// Conversion result: 8-bit (no ADCH) or 10-bit, bipolar when BIN is set.
    fn convert(&self, cx: &Cx) -> u16 {
        let vref = self.vref.max(0.01);
        if self.c.adch.is_none() {
            return ((self.sample * 256.0) / vref).floor().clamp(0.0, 255.0) as u16;
        }
        let bipolar = self.diff && (self.c.diff_signed || self.c.bin != 0 && self.c.adcsrb.is_some_and(|a| cx.cpu.data[a as usize] & self.c.bin != 0));
        if bipolar {
            let v = ((self.sample * 512.0) / vref).floor().clamp(-512.0, 511.0) as i32;
            (v as u16) & 0x3ff
        } else {
            ((self.sample * 1024.0) / vref).floor().clamp(0.0, 1023.0) as u16
        }
    }
}

impl Peripheral for Adc {
    fn name(&self) -> &str {
        "ADC"
    }

    fn read(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let v = cx.cpu.data[addr as usize];
        if addr == self.c.adcl && self.c.adch.is_some() {
            self.locked = true;
        } else if Some(addr) == self.c.adch {
            self.locked = false;
        }
        if addr == self.c.adcsra && self.busy { v | ADSC } else { v }
    }

    fn peek(&mut self, addr: u16, cx: &mut Cx) -> u8 {
        let v = cx.cpu.data[addr as usize];
        if addr == self.c.adcsra && self.busy { v | ADSC } else { v }
    }

    fn write(&mut self, addr: u16, v: u8, cx: &mut Cx) {
        let a = addr as usize;
        if addr == self.c.adcl || Some(addr) == self.c.adch {
            return; // read-only result
        }
        if addr == self.c.admux || (Some(addr) == self.c.adcsrb && self.c.adcsrb_owned) {
            let mask = if addr == self.c.admux { self.c.admux_mask } else { self.c.adcsrb_mask };
            cx.cpu.data[a] = v & mask;
            if self.c.notify {
                cx.sys.events.push_back(crate::avr::machine::Event::RegWritten(addr));
            }
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
        if self.c.notify && (old ^ nv) & ADEN != 0 {
            cx.sys.events.push_back(crate::avr::machine::Event::RegWritten(addr));
        }
        self.update_irq(cx);
    }

    fn on_event(&mut self, _tag: u8, _cycle: u64, cx: &mut Cx) {
        let result = self.convert(cx);
        if !self.locked {
            let adlar_reg = if self.c.adlar_srb { self.c.adcsrb.unwrap_or(self.c.admux) } else { self.c.admux };
            let left = self.c.adlar != 0 && cx.cpu.data[adlar_reg as usize] & self.c.adlar != 0;
            match self.c.adch {
                None => cx.cpu.data[self.c.adcl as usize] = result as u8,
                Some(adch) => {
                    let (h, l) = if left { ((result >> 2) as u8, ((result & 3) << 6) as u8) } else { ((result >> 8) as u8, result as u8) };
                    cx.cpu.data[self.c.adcl as usize] = l;
                    cx.cpu.data[adch as usize] = h;
                }
            }
        }
        cx.cpu.data[self.c.adcsra as usize] |= ADIF;
        self.busy = false;
        let sra = self.sra(cx);
        let free_running = sra & ADATE != 0 && self.trigger_select(cx) == 0 && sra & ADEN != 0;
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
        let ts = self.trigger_select(cx);
        if self.c.adcsrb.is_some() && ts != 0 && self.c.triggers[ts] == Some(trigger) {
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
        self.locked = false;
        self.power_reduced = false;
        self.clock_stopped = false;
        self.update_irq(cx);
    }

    fn inspect(&mut self, cx: &mut Cx) -> Vec<(String, String)> {
        let mux = self.mux(cx);
        let sra = self.sra(cx);
        let input = match self.c.inputs.get(mux).copied().flatten() {
            Some(AdcInput::Pin(g)) => format!("ADC{mux} ({})", cx.sys.pins[g].name),
            Some(AdcInput::Diff(p, n, g)) => format!("{} - {} x{g}", cx.sys.pins[p].name, cx.sys.pins[n].name),
            Some(AdcInput::Temp(_)) => "Temperature sensor".into(),
            Some(AdcInput::Volts(v)) => format!("Internal {v:.2} V"),
            None => "Reserved".into(),
        };
        let (v, _) = self.input(cx);
        vec![
            ("State".into(), if self.busy { "Converting" } else if sra & ADEN != 0 { "Idle" } else { "Disabled" }.into()),
            ("Channel".into(), input),
            ("Input (V)".into(), format!("{v:.3}")),
            ("Vref (V)".into(), format!("{:.2}", self.reference(cx))),
            ("ADC clock div".into(), ADPS_DIV[(sra & 7) as usize].to_string()),
        ]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
