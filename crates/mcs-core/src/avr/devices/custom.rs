//! User-defined ("custom") microcontrollers: a [`CustomMcuConfig`] chooses memory sizes and the
//! number of ports, timers, USARTs, SPIs, TWIs and ADC channels, and [`CustomMcuConfig::build`]
//! generates a complete [`AvrDeviceSpec`] for it.
//!
//! Register layout template: Microchip DS2549 "ATmega640/1280/1281/2560/2561" register summary
//! (the largest classic AVR, a superset of the ATmega328P map). Instances that exist on the
//! ATmega2560 sit at their 2560 addresses (ports A-G at 0x20.., H/J/K/L at 0x100.., TIFR0-5 at
//! 0x35.., TIMSK0-5 at 0x6E.., TC0 0x44, TC1 0x80, TC3 0x90, TC4 0xA0, TC2 0xB0, TC5 0x120, TWI
//! 0xB8, USART0-3 0xC0/0xC8/0xD0/0x130, SPI 0x4C, ADC 0x78.., EECR 0x3F.., RAMPZ 0x5B, EIND 0x5C).
//! Instances beyond the 2560's are allocated sequentially in extended I/O starting at 0x140 and
//! `sram_start` follows the end of the extended I/O (rounded up to 0x100, at least 0x200).
//!
//! Deliberate simplifications (documented, not hidden):
//! * Only the PRR0 bits that exist on the ATmega2560 gate peripherals (TWI, TC0-2, SPI0, USART0,
//!   ADC); PRR1 and further instances are never power-gated.
//! * Timers keep the A/B output compare channels of the simulator's timer model (no OCnC).
//! * ADC: MUX4:0 select channels 0..=29 (30 = 1.1 V bandgap, 31 = GND); there is no MUX5.
//! * RESET, XTAL1/XTAL2, VCC, GND, AVCC and AREF are dedicated pins (no GPIO shares them).
//! * The device signature is synthetic (`1E FF FF`); the part does not exist.
//!
//! Alternate pin functions are dealt out round-robin over the GPIOs in a fixed order (ADC
//! channels first, so ADCn sits on GPIO n, then AIN, INT, USART, SPI, TWI, timer pins); functions
//! only share a pin once every GPIO already carries one, as on real AVRs.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::super::device::*;
use super::super::isa::feature;

pub const MAX_FLASH_BYTES: u32 = 8 << 20;
pub const MAX_PORTS: u8 = 31;
pub const MAX_ADC_CHANNELS: u8 = 30;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomMcuConfig {
    /// Registry id: lower case, starts with `custom-`.
    pub id: String,
    pub name: String,
    /// Flash bytes (even, 64 ..= 8 MiB: the program counter is 22 bits wide).
    pub flash_size: u32,
    pub sram_size: u32,
    /// 0 = no EEPROM; at most 65535 (16-bit EEAR).
    pub eeprom_size: u32,
    /// 8-pin GPIO ports A, B, C ... (no `I`), 1..=31.
    pub ports: u8,
    pub ext_interrupts: u8,
    pub timers8: u8,
    pub timers16: u8,
    pub usarts: u8,
    pub spis: u8,
    pub twis: u8,
    /// 0 = no ADC, else 1..=30 pin channels.
    pub adc_channels: u8,
    pub analog_comparator: bool,
    pub hardware_multiplier: bool,
    /// "DIP" or "SOIC".
    pub package: String,
    pub internal_hz: f64,
    pub max_hz: f64,
    pub vcc: f64,
}

impl Default for CustomMcuConfig {
    /// ATmega328P-like.
    fn default() -> Self {
        Self {
            id: "custom-mcu".into(),
            name: "Custom MCU".into(),
            flash_size: 32768,
            sram_size: 2048,
            eeprom_size: 1024,
            ports: 3,
            ext_interrupts: 2,
            timers8: 2,
            timers16: 1,
            usarts: 1,
            spis: 1,
            twis: 1,
            adc_channels: 6,
            analog_comparator: true,
            hardware_multiplier: true,
            package: "DIP".into(),
            internal_hz: 8e6,
            max_hz: 20e6,
            vcc: 5.0,
        }
    }
}

/// Letters used for port names: A..Z without I (like Atmel). Ports 26.. are `AA`, `AB`, ...
pub fn port_name(i: usize) -> String {
    const L: &[u8] = b"ABCDEFGHJKLMNOPQRSTUVWXYZ";
    if i < L.len() {
        (L[i] as char).to_string()
    } else {
        format!("A{}", L[i - L.len()] as char)
    }
}

/// Timer numbers: 8-bit timers take 0, 2, then 6.. ; 16-bit timers take 1, 3, 4, 5, then the next
/// free numbers (like the 2560's TC0..TC5). Returns (number, is16bit) sorted by number.
pub fn timer_numbers(timers8: u8, timers16: u8) -> Vec<(u8, bool)> {
    let mut next_extra = 6u8;
    let mut out = Vec::new();
    for (count, wide, fixed) in [(timers8, false, &[0u8, 2][..]), (timers16, true, &[1u8, 3, 4, 5][..])] {
        for i in 0..count as usize {
            let n = match fixed.get(i) {
                Some(&n) => n,
                None => {
                    let n = next_extra;
                    next_extra += 1;
                    n
                }
            };
            out.push((n, wide));
        }
    }
    out.sort_unstable();
    out
}

/// DIDR register holding the digital-input-disable bit of ADC channel `c`.
pub fn didr_name(c: usize) -> String {
    match c / 8 {
        0 => "DIDR0".into(),
        1 => "DIDR2".into(),
        k => format!("DIDR{}", k + 1),
    }
}

fn sx(k: usize) -> String {
    if k == 0 {
        String::new()
    } else {
        k.to_string()
    }
}

/// Register-name helper for single-instance registers that gain a number from instance 1 on.
fn ix(base: &str, k: usize) -> String {
    format!("{base}{}", sx(k))
}

fn reg(name: &str, addr: u16, group: &str, desc: &str, bits: Vec<BitFieldSpec>) -> IoRegisterSpec {
    IoRegisterSpec { name: name.into(), addr, reset: 0, group: group.into(), desc: desc.into(), bits, access: RegisterAccess::Rw }
}

fn reset(mut r: IoRegisterSpec, v: u8) -> IoRegisterSpec {
    r.reset = v;
    r
}

fn ro(mut r: IoRegisterSpec) -> IoRegisterSpec {
    r.access = RegisterAccess::R;
    r
}

/// Single-bit fields named `prefix{i}` for the bits set in `mask`, MSB first.
fn nbits(prefix: &str, mask: u8) -> Vec<BitFieldSpec> {
    (0..8).rev().filter(|i| mask & (1 << i) != 0).map(|i| field(&format!("{prefix}{i}"), 1 << i, "")).collect()
}

fn b(name: &str, mask: u8, desc: &str) -> BitFieldSpec {
    field(name, mask, desc)
}

/// Extended-I/O allocator (sequential from 0x140).
struct Ext {
    next: u32,
}

impl Ext {
    fn alloc(&mut self, n: u32) -> u16 {
        let a = self.next;
        self.next += n;
        a.min(0xffff) as u16
    }
}

impl CustomMcuConfig {
    /// A tiny part: one port, nothing else, 64 bytes of flash and 32 bytes of SRAM.
    pub fn tiny() -> Self {
        Self {
            id: "custom-tiny".into(),
            name: "Tiny".into(),
            flash_size: 64,
            sram_size: 32,
            eeprom_size: 0,
            ports: 1,
            ext_interrupts: 0,
            timers8: 0,
            timers16: 0,
            usarts: 0,
            spis: 0,
            twis: 0,
            adc_channels: 0,
            analog_comparator: false,
            hardware_multiplier: false,
            ..Self::default()
        }
    }

    /// The largest part the architecture allows (8 MiB flash, every peripheral count at its cap
    /// of the maxima used by the tests, the most SRAM that fits the 16-bit data space).
    pub fn huge() -> Self {
        let mut c = Self {
            id: "custom-huge".into(),
            name: "Huge".into(),
            flash_size: MAX_FLASH_BYTES,
            sram_size: 32,
            eeprom_size: 65535,
            ports: MAX_PORTS,
            ext_interrupts: 16,
            timers8: 8,
            timers16: 8,
            usarts: 8,
            spis: 4,
            twis: 4,
            adc_channels: MAX_ADC_CHANNELS,
            ..Self::default()
        };
        if let Ok(s) = c.build() {
            c.sram_size = 0xffff - s.sram_start as u32;
        }
        c
    }

    fn gpio_count(&self) -> usize {
        self.ports as usize * 8
    }

    fn check(&self) -> Result<(), String> {
        let id_ok = self.id.len() > "custom-".len()
            && self.id.len() <= 48
            && self.id.starts_with("custom-")
            && self.id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_');
        if !id_ok {
            return Err(format!("Device id '{}' must be 8-48 characters of a-z, 0-9, '-' or '_' and start with 'custom-'", self.id));
        }
        let n = self.name.trim();
        if n.is_empty() || n.chars().count() > 48 || n.chars().any(char::is_control) {
            return Err("Device name must be 1-48 printable characters".into());
        }
        if self.flash_size < 64 || !self.flash_size.is_multiple_of(2) {
            return Err(format!("Flash size {} B must be even and at least 64 B (instructions are 16-bit words and the interrupt table needs room)", self.flash_size));
        }
        if self.flash_size > MAX_FLASH_BYTES {
            return Err(format!("Flash size {} B exceeds 8 MiB: the AVR program counter is 22 bits wide (4 Mi words)", self.flash_size));
        }
        if self.sram_size < 32 {
            return Err(format!("SRAM size {} B is below the 32 B minimum (the 32 CPU registers sit below SRAM; programs need a stack)", self.sram_size));
        }
        if self.eeprom_size > 65535 {
            return Err(format!("EEPROM size {} B exceeds 65535 B: the EEPROM address register EEAR and the size field are 16 bits wide", self.eeprom_size));
        }
        if !(1..=MAX_PORTS).contains(&self.ports) {
            return Err(format!("Port count {} must be 1..={MAX_PORTS}: GPIO indices are 8 bits (at most 255 GPIOs, 8 per port)", self.ports));
        }
        // Peripheral counts have no cap of their own: the real limits (255 interrupt vectors,
        // registers fitting the 16-bit data space) are checked when the spec is built.
        if self.adc_channels > MAX_ADC_CHANNELS {
            return Err(format!("ADC channel count {} exceeds {MAX_ADC_CHANNELS}: ADMUX.MUX4:0 has 32 codes and 30/31 select the 1.1 V bandgap and GND", self.adc_channels));
        }
        if !matches!(self.package.as_str(), "DIP" | "SOIC") {
            return Err(format!("Package '{}' must be \"DIP\" or \"SOIC\"", self.package));
        }
        if !(self.internal_hz >= 1e3 && self.internal_hz <= 1e9) {
            return Err("Internal oscillator must be between 1 kHz and 1 GHz".into());
        }
        if !(self.max_hz >= 1e3 && self.max_hz <= 1e9) {
            return Err("Maximum clock must be between 1 kHz and 1 GHz".into());
        }
        if !(1.8..=5.5).contains(&self.vcc) {
            return Err(format!("VCC {} V must be within the operating range 1.8-5.5 V", self.vcc));
        }
        Ok(())
    }

    /// Checks every limit; the messages name the limit and why it exists.
    pub fn validate(&self) -> Result<(), String> {
        self.build().map(|_| ())
    }

    /// Generates the device description.
    pub fn build(&self) -> Result<AvrDeviceSpec, String> {
        self.check()?;
        let ports = self.ports as usize;
        let gpios = self.gpio_count();
        let timers = timer_numbers(self.timers8, self.timers16);
        let has_t = |n: u8| timers.iter().any(|t| t.0 == n);
        let (usarts, spis, twis) = (self.usarts as usize, self.spis as usize, self.twis as usize);
        let adc = self.adc_channels as usize;
        let ints = self.ext_interrupts as usize;
        let has_eeprom = self.eeprom_size > 0;
        let flash = self.flash_size;
        let has_boot = flash >= 8192;
        let pnames: Vec<String> = (0..ports).map(port_name).collect();

        // ----- pins and alternate functions -----
        let mut funcs: Vec<Vec<String>> = vec![Vec::new(); gpios];
        let mut cursor = 0usize;
        let mut give = |s: String| {
            funcs[cursor % gpios].push(s);
            cursor += 1;
        };
        for c in 0..adc {
            give(format!("ADC{c}"));
        }
        if self.analog_comparator {
            give("AIN0".into());
            give("AIN1".into());
        }
        for k in 0..ints {
            give(format!("INT{k}"));
        }
        for k in 0..usarts {
            give(format!("RXD{k}"));
            give(format!("TXD{k}"));
        }
        for k in 0..spis {
            for f in ["SS", "MOSI", "MISO", "SCK"] {
                give(ix(f, k));
            }
        }
        for k in 0..twis {
            give(ix("SDA", k));
            give(ix("SCL", k));
        }
        for &(n, wide) in &timers {
            if n != 2 {
                give(format!("T{n}"));
            }
            give(format!("OC{n}A"));
            give(format!("OC{n}B"));
            if wide {
                give(format!("ICP{n}"));
            }
        }
        for (g, f) in funcs.iter_mut().enumerate() {
            f.push(format!("PCINT{g}"));
        }
        // DIDR bit of the first ADC channel on each GPIO.
        let mut pins: Vec<PinSpec> = Vec::new();
        let power = |name: &str, kind: PinKind, functions: Vec<String>| PinSpec { number: 0, name: name.into(), kind, gpio: None, functions };
        pins.push(power("RESET", PinKind::Io, vec!["RESET".into()]));
        pins.push(power("VCC", PinKind::Vcc, vec![]));
        pins.push(power("GND", PinKind::Gnd, vec![]));
        pins.push(power("XTAL1", PinKind::Io, vec!["XTAL1".into()]));
        pins.push(power("XTAL2", PinKind::Io, vec!["XTAL2".into()]));
        pins.push(power("AVCC", PinKind::Ref, vec![]));
        if adc > 0 {
            pins.push(power("AREF", PinKind::Ref, vec!["AREF".into()]));
        }
        for (g, f) in funcs.into_iter().enumerate() {
            pins.push(PinSpec { number: 0, name: format!("P{}{}", pnames[g / 8], g % 8), kind: PinKind::Io, gpio: Some(g as u8), functions: f });
        }
        if !pins.len().is_multiple_of(2) && pins.len() < 255 {
            pins.push(power("GND", PinKind::Gnd, vec![]));
        }
        for (i, p) in pins.iter_mut().enumerate() {
            p.number = (i + 1) as u8;
        }
        let package = format!("{}-{}", if self.package == "DIP" { "PDIP" } else { "SOIC" }, pins.len());

        // ----- registers -----
        let mut ext = Ext { next: 0x140 };
        let mut r: Vec<IoRegisterSpec> = Vec::new();

        // Ports.
        for (i, l) in pnames.iter().enumerate() {
            let base = match i {
                0..=6 => 0x20 + 3 * i as u16,
                7..=10 => 0x100 + 3 * (i as u16 - 7),
                _ => ext.alloc(3),
            };
            let grp = format!("PORT{l}");
            r.push(reg(&format!("PIN{l}"), base, &grp, &format!("Port {l} Input Pins (write 1 toggles PORT{l} bit)"), nbits(&format!("PIN{l}"), 0xff)));
            r.push(reg(&format!("DDR{l}"), base + 1, &grp, &format!("Port {l} Data Direction Register"), nbits(&format!("DD{l}"), 0xff)));
            r.push(reg(&format!("PORT{l}"), base + 2, &grp, &format!("Port {l} Data Register"), nbits(&format!("PORT{l}"), 0xff)));
        }

        // Timers.
        let mut sync_timer = false;
        for &(n, wide) in &timers {
            let grp = format!("TC{n}");
            let tifr = if n < 6 { 0x35 + n as u16 } else { ext.alloc(1) };
            let timsk = if n < 6 { 0x6e + n as u16 } else { ext.alloc(1) };
            let base = match n {
                0 => 0x44,
                1 => 0x80,
                2 => 0xb0,
                3 => 0x90,
                4 => 0xa0,
                5 => 0x120,
                _ => ext.alloc(if wide { 12 } else { 5 }),
            };
            sync_timer |= n != 2;
            let wgm_a = vec![
                b(&format!("COM{n}A"), 0xc0, "Compare Output Mode A"),
                b(&format!("COM{n}B"), 0x30, "Compare Output Mode B"),
                b(&format!("WGM{n}1"), 0x02, "Waveform Generation Mode bit 1"),
                b(&format!("WGM{n}0"), 0x01, "Waveform Generation Mode bit 0"),
            ];
            let cs_desc = if n == 2 {
                "Clock Select (0 stop, 1 /1, 2 /8, 3 /32, 4 /64, 5 /128, 6 /256, 7 /1024)"
            } else {
                "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 Tn falling, 7 Tn rising)"
            };
            r.push(reg(&format!("TCCR{n}A"), base, &grp, &format!("Timer/Counter{n} Control Register A"), wgm_a));
            if wide {
                r.push(reg(&format!("TCCR{n}B"), base + 1, &grp, &format!("Timer/Counter{n} Control Register B"), vec![
                    b(&format!("ICNC{n}"), 0x80, "Input Capture Noise Canceler"), b(&format!("ICES{n}"), 0x40, "Input Capture Edge Select (1 = rising)"),
                    b(&format!("WGM{n}3"), 0x10, "Waveform Generation Mode bit 3"), b(&format!("WGM{n}2"), 0x08, "Waveform Generation Mode bit 2"), b(&format!("CS{n}"), 0x07, cs_desc),
                ]));
                r.push(reg(&format!("TCCR{n}C"), base + 2, &grp, &format!("Timer/Counter{n} Control Register C"), vec![b(&format!("FOC{n}A"), 0x80, "Force Output Compare A"), b(&format!("FOC{n}B"), 0x40, "Force Output Compare B")]));
                r.push(reg(&format!("TCNT{n}L"), base + 4, &grp, &format!("Timer/Counter{n} Low Byte"), vec![]));
                r.push(reg(&format!("TCNT{n}H"), base + 5, &grp, &format!("Timer/Counter{n} High Byte"), vec![]));
                r.push(reg(&format!("ICR{n}L"), base + 6, &grp, &format!("Input Capture Register {n} Low Byte"), vec![]));
                r.push(reg(&format!("ICR{n}H"), base + 7, &grp, &format!("Input Capture Register {n} High Byte"), vec![]));
                for (i, ch) in ["A", "B"].into_iter().enumerate() {
                    r.push(reg(&format!("OCR{n}{ch}L"), base + 8 + 2 * i as u16, &grp, &format!("Output Compare Register {n} {ch} Low Byte"), vec![]));
                    r.push(reg(&format!("OCR{n}{ch}H"), base + 9 + 2 * i as u16, &grp, &format!("Output Compare Register {n} {ch} High Byte"), vec![]));
                }
                r.push(reg(&format!("TIFR{n}"), tifr, &grp, &format!("Timer/Counter{n} Interrupt Flag Register"), vec![
                    b(&format!("ICF{n}"), 0x20, "Input Capture Flag"), b(&format!("OCF{n}B"), 0x04, "Output Compare B Match Flag"), b(&format!("OCF{n}A"), 0x02, "Output Compare A Match Flag"), b(&format!("TOV{n}"), 0x01, "Overflow Flag"),
                ]));
                r.push(reg(&format!("TIMSK{n}"), timsk, &grp, &format!("Timer/Counter{n} Interrupt Mask Register"), vec![
                    b(&format!("ICIE{n}"), 0x20, "Input Capture Interrupt Enable"), b(&format!("OCIE{n}B"), 0x04, "Output Compare B Match Interrupt Enable"), b(&format!("OCIE{n}A"), 0x02, "Output Compare A Match Interrupt Enable"), b(&format!("TOIE{n}"), 0x01, "Overflow Interrupt Enable"),
                ]));
            } else {
                r.push(reg(&format!("TCCR{n}B"), base + 1, &grp, &format!("Timer/Counter{n} Control Register B"), vec![
                    b(&format!("FOC{n}A"), 0x80, "Force Output Compare A"), b(&format!("FOC{n}B"), 0x40, "Force Output Compare B"), b(&format!("WGM{n}2"), 0x08, "Waveform Generation Mode bit 2"), b(&format!("CS{n}"), 0x07, cs_desc),
                ]));
                r.push(reg(&format!("TCNT{n}"), base + 2, &grp, &format!("Timer/Counter{n}"), vec![]));
                r.push(reg(&format!("OCR{n}A"), base + 3, &grp, &format!("Output Compare Register {n} A"), vec![]));
                r.push(reg(&format!("OCR{n}B"), base + 4, &grp, &format!("Output Compare Register {n} B"), vec![]));
                r.push(reg(&format!("TIFR{n}"), tifr, &grp, &format!("Timer/Counter{n} Interrupt Flag Register"), vec![
                    b(&format!("OCF{n}B"), 0x04, "Output Compare B Match Flag"), b(&format!("OCF{n}A"), 0x02, "Output Compare A Match Flag"), b(&format!("TOV{n}"), 0x01, "Overflow Flag"),
                ]));
                r.push(reg(&format!("TIMSK{n}"), timsk, &grp, &format!("Timer/Counter{n} Interrupt Mask Register"), vec![
                    b(&format!("OCIE{n}B"), 0x04, "Output Compare B Match Interrupt Enable"), b(&format!("OCIE{n}A"), 0x02, "Output Compare A Match Interrupt Enable"), b(&format!("TOIE{n}"), 0x01, "Overflow Interrupt Enable"),
                ]));
                if n == 2 {
                    r.push(reg("ASSR", 0xb6, &grp, "Asynchronous Status Register", vec![
                        b("EXCLK", 0x40, "Enable External Clock Input"), b("AS2", 0x20, "Asynchronous Timer/Counter2"), b("TCN2UB", 0x10, "TCNT2 Update Busy"),
                        b("OCR2AUB", 0x08, "OCR2A Update Busy"), b("OCR2BUB", 0x04, "OCR2B Update Busy"), b("TCR2AUB", 0x02, "TCCR2A Update Busy"), b("TCR2BUB", 0x01, "TCCR2B Update Busy"),
                    ]));
                }
            }
        }
        if !timers.is_empty() {
            let mut bits = vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode")];
            if has_t(2) {
                bits.push(b("PSRASY", 0x02, "Prescaler Reset Timer/Counter2"));
            }
            if sync_timer {
                bits.push(b("PSRSYNC", 0x01, "Prescaler Reset synchronous Timer/Counters"));
            }
            r.push(reg("GTCCR", 0x43, &format!("TC{}", timers[0].0), "General Timer/Counter Control Register", bits));
        }

        // External and pin-change interrupts.
        let mut ex = |std_addr: Option<u16>| std_addr.unwrap_or_else(|| ext.alloc(1));
        if ints > 0 {
            for j in 0..ints.div_ceil(4) {
                let l = (b'A' + j as u8) as char;
                let addr = ex((j < 2).then_some(0x69 + j as u16));
                let bits = (j * 4..ints.min(j * 4 + 4)).map(|k| b(&format!("ISC{k}"), 3 << ((k % 4) * 2), "Interrupt Sense Control (00 low, 01 any, 10 falling, 11 rising)")).collect();
                r.push(reg(&format!("EICR{l}"), addr, "EXINT", &format!("External Interrupt Control Register {l}"), bits));
            }
            for m in 0..ints.div_ceil(8) {
                let ks = m * 8..ints.min(m * 8 + 8);
                let flags = ex((m == 0).then_some(0x3c));
                r.push(reg(&ix("EIFR", m), flags, "EXINT", "External Interrupt Flag Register", ks.clone().rev().map(|k| b(&format!("INTF{k}"), 1 << (k % 8), "External Interrupt Flag")).collect()));
                let mask = ex((m == 0).then_some(0x3d));
                r.push(reg(&ix("EIMSK", m), mask, "EXINT", "External Interrupt Mask Register", ks.rev().map(|k| b(&format!("INT{k}"), 1 << (k % 8), "External Interrupt Request Enable")).collect()));
            }
        }
        for m in 0..ports.div_ceil(8) {
            let gs = m * 8..ports.min(m * 8 + 8);
            let flags = ex((m == 0).then_some(0x3b));
            r.push(reg(&ix("PCIFR", m), flags, "EXINT", "Pin Change Interrupt Flag Register", gs.clone().rev().map(|g| b(&format!("PCIF{g}"), 1 << (g % 8), "Pin Change Interrupt Flag")).collect()));
            let ctl = ex((m == 0).then_some(0x68));
            r.push(reg(&ix("PCICR", m), ctl, "EXINT", "Pin Change Interrupt Control Register", gs.rev().map(|g| b(&format!("PCIE{g}"), 1 << (g % 8), "Pin Change Interrupt Enable")).collect()));
        }
        for (g, pn) in pnames.iter().enumerate() {
            let addr = ex((g < 3).then_some(0x6b + g as u16));
            r.push(reg(&format!("PCMSK{g}"), addr, "EXINT", &format!("Pin Change Mask Register {g} (P{pn}7..P{pn}0)"), (g * 8..g * 8 + 8).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i % 8), "")).collect()));
        }

        // EEPROM.
        if has_eeprom {
            r.push(reg("EECR", 0x3f, "EEPROM", "EEPROM Control Register", vec![
                b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
                b("EEMPE", 0x04, "EEPROM Master Write Enable"), b("EEPE", 0x02, "EEPROM Write Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
            ]));
            r.push(reg("EEDR", 0x40, "EEPROM", "EEPROM Data Register", vec![]));
            r.push(reg("EEARL", 0x41, "EEPROM", "EEPROM Address Register Low Byte", vec![]));
            r.push(reg("EEARH", 0x42, "EEPROM", "EEPROM Address Register High Byte", vec![]));
        }

        // CPU.
        r.push(reg("GPIOR0", 0x3e, "CPU", "General Purpose I/O Register 0", vec![]));
        r.push(reg("GPIOR1", 0x4a, "CPU", "General Purpose I/O Register 1", vec![]));
        r.push(reg("GPIOR2", 0x4b, "CPU", "General Purpose I/O Register 2", vec![]));
        r.push(reg("SMCR", 0x53, "CPU", "Sleep Mode Control Register", vec![b("SM", 0x0e, "Sleep Mode (000 idle, 001 ADC NR, 010 power-down, 011 power-save, 110 standby, 111 ext. standby)"), b("SE", 0x01, "Sleep Enable")]));
        r.push(reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]));
        r.push(reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("BODS", 0x40, "BOD Sleep"), b("BODSE", 0x20, "BOD Sleep Enable"), b("PUD", 0x10, "Pull-up Disable"), b("IVSEL", 0x02, "Interrupt Vector Select (boot section)"), b("IVCE", 0x01, "Interrupt Vector Change Enable"),
        ]));
        if has_boot {
            r.push(reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![
                b("SPMIE", 0x80, "SPM Interrupt Enable"), b("RWWSB", 0x40, "Read-While-Write Section Busy"), b("SIGRD", 0x20, "Signature Row Read"), b("RWWSRE", 0x10, "RWW Section Read Enable"),
                b("BLBSET", 0x08, "Boot Lock Bit Set"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Store Program Memory Enable"),
            ]));
        }
        if flash > 65536 {
            r.push(reg("RAMPZ", 0x5b, "CPU", "Extended Z-pointer Register (flash bits 23:16 for ELPM/SPM)", nbits("RAMPZ", 0xff)));
        }
        if flash > 131072 {
            r.push(reg("EIND", 0x5c, "CPU", "Extended Indirect Register (bits 21:16 of EIJMP/EICALL)", nbits("EIND", 0xff)));
        }
        r.push(reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]));
        r.push(reg("SPH", 0x5e, "CPU", "Stack Pointer High Byte", vec![]));
        r.push(reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )));
        r.push(reg("WDTCSR", 0x60, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]));
        r.push(reg("CLKPR", 0x61, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]));
        r.push(reg("OSCCAL", 0x66, "CPU", "Oscillator Calibration Register", vec![]));
        let mut prr = vec![];
        if twis > 0 {
            prr.push(b("PRTWI", 0x80, "Power Reduction TWI"));
        }
        if has_t(2) {
            prr.push(b("PRTIM2", 0x40, "Power Reduction Timer/Counter2"));
        }
        if has_t(0) {
            prr.push(b("PRTIM0", 0x20, "Power Reduction Timer/Counter0"));
        }
        if has_t(1) {
            prr.push(b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"));
        }
        if spis > 0 {
            prr.push(b("PRSPI", 0x04, "Power Reduction SPI"));
        }
        if usarts > 0 {
            prr.push(b("PRUSART0", 0x02, "Power Reduction USART0"));
        }
        if adc > 0 {
            prr.push(b("PRADC", 0x01, "Power Reduction ADC"));
        }
        r.push(reg("PRR0", 0x64, "CPU", "Power Reduction Register 0", prr));

        // SPI.
        for k in 0..spis {
            let base = if k == 0 { 0x4c } else { ext.alloc(3) };
            let (s, grp) = (sx(k), ix("SPI", k));
            r.push(reg(&format!("SPCR{s}"), base, &grp, "SPI Control Register", vec![
                b(&format!("SPIE{s}"), 0x80, "SPI Interrupt Enable"), b(&format!("SPE{s}"), 0x40, "SPI Enable"), b(&format!("DORD{s}"), 0x20, "Data Order (1 = LSB first)"), b(&format!("MSTR{s}"), 0x10, "Master/Slave Select"),
                b(&format!("CPOL{s}"), 0x08, "Clock Polarity"), b(&format!("CPHA{s}"), 0x04, "Clock Phase"), b(&format!("SPR{s}"), 0x03, "SPI Clock Rate Select (fosc/4, /16, /64, /128)"),
            ]));
            r.push(reg(&format!("SPSR{s}"), base + 1, &grp, "SPI Status Register", vec![b(&format!("SPIF{s}"), 0x80, "SPI Interrupt Flag"), b(&format!("WCOL{s}"), 0x40, "Write Collision Flag"), b(&format!("SPI2X{s}"), 0x01, "Double SPI Speed")]));
            r.push(reg(&format!("SPDR{s}"), base + 2, &grp, "SPI Data Register", vec![]));
        }
        // Analog comparator.
        if self.analog_comparator {
            r.push(reg("ACSR", 0x50, "AC", "Analog Comparator Control and Status Register", vec![
                b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select (1.1 V on the positive input)"), b("ACO", 0x20, "Analog Comparator Output"),
                b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Input Capture Enable (Timer1)"),
                b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
            ]));
            r.push(reg("DIDR1", 0x7f, "AC", "Digital Input Disable Register 1", vec![b("AIN1D", 0x02, "AIN1 Digital Input Disable"), b("AIN0D", 0x01, "AIN0 Digital Input Disable")]));
        }
        // ADC.
        if adc > 0 {
            r.push(ro(reg("ADCL", 0x78, "ADC", "ADC Data Register Low Byte (read first)", vec![])));
            r.push(ro(reg("ADCH", 0x79, "ADC", "ADC Data Register High Byte", vec![])));
            r.push(reg("ADCSRA", 0x7a, "ADC", "ADC Control and Status Register A", vec![
                b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
                b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
            ]));
            let mut adcsrb = vec![];
            if self.analog_comparator {
                adcsrb.push(b("ACME", 0x40, "Analog Comparator Multiplexer Enable"));
            }
            adcsrb.push(b("ADTS", 0x07, "ADC Auto Trigger Source"));
            r.push(reg("ADCSRB", 0x7b, "ADC", "ADC Control and Status Register B", adcsrb));
            r.push(reg("ADMUX", 0x7c, "ADC", "ADC Multiplexer Selection Register", vec![
                b("REFS", 0xc0, "Reference Selection (00 AREF, 01 AVCC, 11 internal 1.1 V)"), b("ADLAR", 0x20, "ADC Left Adjust Result"),
                b("MUX", 0x1f, "Analog Channel Selection (0-29 ADCn, 30 1.1 V, 31 GND)"),
            ]));
            for d in 0..adc.div_ceil(8) {
                let name = didr_name(d * 8);
                let addr = match d {
                    0 => 0x7e,
                    1 => 0x7d,
                    _ => ext.alloc(1),
                };
                r.push(reg(&name, addr, "ADC", "Digital Input Disable Register", (d * 8..adc.min(d * 8 + 8)).rev().map(|c| field(&format!("ADC{c}D"), 1 << (c % 8), "")).collect()));
            }
        }
        // TWI.
        for k in 0..twis {
            let base = if k == 0 { 0xb8 } else { ext.alloc(6) };
            let (s, grp) = (sx(k), ix("TWI", k));
            r.push(reg(&format!("TWBR{s}"), base, &grp, "TWI Bit Rate Register", vec![]));
            r.push(reset(reg(&format!("TWSR{s}"), base + 1, &grp, "TWI Status Register", vec![b(&format!("TWS{s}"), 0xf8, "TWI Status"), b(&format!("TWPS{s}"), 0x03, "TWI Prescaler (1, 4, 16, 64)")]), 0xf8));
            r.push(reset(reg(&format!("TWAR{s}"), base + 2, &grp, "TWI (Slave) Address Register", vec![b(&format!("TWA{s}"), 0xfe, "TWI Slave Address"), b(&format!("TWGCE{s}"), 0x01, "TWI General Call Recognition Enable")]), 0xfe));
            r.push(reset(reg(&format!("TWDR{s}"), base + 3, &grp, "TWI Data Register", vec![]), 0xff));
            r.push(reg(&format!("TWCR{s}"), base + 4, &grp, "TWI Control Register", vec![
                b(&format!("TWINT{s}"), 0x80, "TWI Interrupt Flag"), b(&format!("TWEA{s}"), 0x40, "TWI Enable Acknowledge"), b(&format!("TWSTA{s}"), 0x20, "TWI START Condition"), b(&format!("TWSTO{s}"), 0x10, "TWI STOP Condition"),
                b(&format!("TWWC{s}"), 0x08, "TWI Write Collision"), b(&format!("TWEN{s}"), 0x04, "TWI Enable"), b(&format!("TWIE{s}"), 0x01, "TWI Interrupt Enable"),
            ]));
            r.push(reg(&format!("TWAMR{s}"), base + 5, &grp, "TWI (Slave) Address Mask Register", vec![]));
        }
        // USART.
        for k in 0..usarts {
            let base = match k {
                0 => 0xc0,
                1 => 0xc8,
                2 => 0xd0,
                3 => 0x130,
                _ => ext.alloc(8),
            };
            let grp = format!("USART{k}");
            r.push(reset(reg(&format!("UCSR{k}A"), base, &grp, "USART Control and Status Register A", vec![
                b(&format!("RXC{k}"), 0x80, "Receive Complete"), b(&format!("TXC{k}"), 0x40, "Transmit Complete"), b(&format!("UDRE{k}"), 0x20, "Data Register Empty"), b(&format!("FE{k}"), 0x10, "Frame Error"),
                b(&format!("DOR{k}"), 0x08, "Data OverRun"), b(&format!("UPE{k}"), 0x04, "Parity Error"), b(&format!("U2X{k}"), 0x02, "Double Transmission Speed"), b(&format!("MPCM{k}"), 0x01, "Multi-processor Communication Mode"),
            ]), 0x20));
            r.push(reg(&format!("UCSR{k}B"), base + 1, &grp, "USART Control and Status Register B", vec![
                b(&format!("RXCIE{k}"), 0x80, "RX Complete Interrupt Enable"), b(&format!("TXCIE{k}"), 0x40, "TX Complete Interrupt Enable"), b(&format!("UDRIE{k}"), 0x20, "Data Register Empty Interrupt Enable"),
                b(&format!("RXEN{k}"), 0x10, "Receiver Enable"), b(&format!("TXEN{k}"), 0x08, "Transmitter Enable"), b(&format!("UCSZ{k}2"), 0x04, "Character Size bit 2"), b(&format!("RXB8{k}"), 0x02, "Receive Data Bit 8"), b(&format!("TXB8{k}"), 0x01, "Transmit Data Bit 8"),
            ]));
            r.push(reset(reg(&format!("UCSR{k}C"), base + 2, &grp, "USART Control and Status Register C", vec![
                b(&format!("UMSEL{k}"), 0xc0, "USART Mode Select (00 asynchronous)"), b(&format!("UPM{k}"), 0x30, "Parity Mode (00 none, 10 even, 11 odd)"), b(&format!("USBS{k}"), 0x08, "Stop Bit Select (1 = 2 stop bits)"),
                b(&format!("UCSZ{k}"), 0x06, "Character Size bits 1:0 (11 = 8 bits)"), b(&format!("UCPOL{k}"), 0x01, "Clock Polarity"),
            ]), 0x06));
            r.push(reg(&format!("UBRR{k}L"), base + 4, &grp, "USART Baud Rate Register Low Byte", vec![]));
            r.push(reg(&format!("UBRR{k}H"), base + 5, &grp, "USART Baud Rate Register High Byte", vec![]));
            r.push(reg(&format!("UDR{k}"), base + 6, &grp, "USART I/O Data Register", vec![]));
        }

        // ----- SRAM placement -----
        if ext.next > 0xff00 {
            return Err("Peripheral registers do not fit the 16-bit data space (reduce the number of ports, timers or serial interfaces)".into());
        }
        let top = r.iter().map(|x| x.addr as u32 + 1).max().unwrap_or(0);
        let sram_start = (top.div_ceil(0x100) * 0x100).max(0x200);
        // The data space is 16 bits; the last byte is left unused so address arithmetic never wraps.
        if sram_start + self.sram_size > 0xffff {
            return Err(format!(
                "SRAM of {} B does not fit: the 16-bit data space holds at most {} B above the {sram_start:#x} bytes of registers and I/O (fewer peripherals leave more room)",
                self.sram_size,
                0xffff - sram_start
            ));
        }
        let ram_end = sram_start + self.sram_size - 1;
        for x in &mut r {
            match x.name.as_str() {
                "SPL" => x.reset = (ram_end & 0xff) as u8,
                "SPH" => x.reset = (ram_end >> 8) as u8,
                _ => {}
            }
        }
        r.sort_by_key(|x| x.addr);

        // ----- interrupt vectors -----
        let mut vecs: Vec<VectorSpec> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut push = |name: String, desc: String| {
            if seen.insert(name.clone()) {
                vecs.push(VectorSpec { index: vecs.len() as u8, name, desc });
            }
        };
        let timer_vecs = |push: &mut dyn FnMut(String, String), n: u8| {
            if let Some(&(_, wide)) = timers.iter().find(|t| t.0 == n) {
                if wide {
                    push(format!("TIMER{n}_CAPT"), format!("Timer/Counter{n} Capture Event"));
                }
                push(format!("TIMER{n}_COMPA"), format!("Timer/Counter{n} Compare Match A"));
                push(format!("TIMER{n}_COMPB"), format!("Timer/Counter{n} Compare Match B"));
                push(format!("TIMER{n}_OVF"), format!("Timer/Counter{n} Overflow"));
            }
        };
        let int_vec = |push: &mut dyn FnMut(String, String), k: usize| push(format!("INT{k}"), format!("External Interrupt Request {k}"));
        let pc_vec = |push: &mut dyn FnMut(String, String), g: usize| push(format!("PCINT{g}"), format!("Pin Change Interrupt Request {g}"));
        let spi_vec = |push: &mut dyn FnMut(String, String), k: usize| push(if k == 0 { "SPI_STC".into() } else { format!("SPI{k}_STC") }, format!("SPI{} Serial Transfer Complete", sx(k)));
        let usart_vec = |push: &mut dyn FnMut(String, String), k: usize| {
            push(format!("USART{k}_RX"), format!("USART{k} Rx Complete"));
            push(format!("USART{k}_UDRE"), format!("USART{k} Data Register Empty"));
            push(format!("USART{k}_TX"), format!("USART{k} Tx Complete"));
        };
        let twi_vec = |push: &mut dyn FnMut(String, String), k: usize| push(ix("TWI", k), format!("2-wire Serial Interface {}", sx(k)).trim_end().to_string());
        // ATmega2560 order (DS2549 table 14-1) for what exists there, everything else appended.
        push("RESET".into(), "External Pin, Power-on Reset, Brown-out Reset and Watchdog System Reset".into());
        (0..ints.min(8)).for_each(|k| int_vec(&mut push, k));
        (0..ports.min(3)).for_each(|g| pc_vec(&mut push, g));
        push("WDT".into(), "Watchdog Time-out Interrupt".into());
        for n in [2, 1, 0] {
            timer_vecs(&mut push, n);
        }
        if spis > 0 {
            spi_vec(&mut push, 0);
        }
        if usarts > 0 {
            usart_vec(&mut push, 0);
        }
        if self.analog_comparator {
            push("ANALOG_COMP".into(), "Analog Comparator".into());
        }
        if adc > 0 {
            push("ADC".into(), "ADC Conversion Complete".into());
        }
        if has_eeprom {
            push("EE_READY".into(), "EEPROM Ready".into());
        }
        timer_vecs(&mut push, 3);
        if usarts > 1 {
            usart_vec(&mut push, 1);
        }
        if twis > 0 {
            twi_vec(&mut push, 0);
        }
        if has_boot {
            push("SPM_READY".into(), "Store Program Memory Ready".into());
        }
        timer_vecs(&mut push, 4);
        timer_vecs(&mut push, 5);
        for k in 2..usarts.min(4) {
            usart_vec(&mut push, k);
        }
        (0..ints).for_each(|k| int_vec(&mut push, k));
        (0..ports).for_each(|g| pc_vec(&mut push, g));
        for &(n, _) in &timers {
            timer_vecs(&mut push, n);
        }
        (0..spis).for_each(|k| spi_vec(&mut push, k));
        (0..usarts).for_each(|k| usart_vec(&mut push, k));
        (0..twis).for_each(|k| twi_vec(&mut push, k));
        let vec_words = if flash > 8192 { 2 } else { 1 };
        if vecs.len() > 255 {
            return Err(format!("{} interrupt vectors exceed the 255 the simulator can index", vecs.len()));
        }
        if vecs.len() as u32 * vec_words * 2 > flash {
            return Err(format!(
                "The interrupt vector table ({} vectors x {vec_words} word(s)) does not fit in {flash} B of flash; increase flash or remove peripherals",
                vecs.len()
            ));
        }

        // ----- fuses (ATmega328P layout) -----
        let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
        let low = FuseByteSpec {
            name: "Low".into(),
            default: 0x62,
            bits: vec![
                f("CKDIV8", 0x80, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
                f("CKOUT", 0x40, "Clock output when programmed (0)"),
                f("SUT", 0x30, "Start-up time select"),
                f("CKSEL", 0x0f, "Clock source (0000 external clock, 0010 internal RC, 0011 internal 128 kHz, 0100-0101 32 kHz crystal, 0110-0111 full-swing crystal, 1000-1111 crystal)"),
            ],
        };
        let mut hi = vec![
            f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
            f("WDTON", 0x10, "Watchdog Timer always on when programmed (0)"),
            f("EESAVE", 0x08, "EEPROM preserved through chip erase when programmed (0)"),
        ];
        if has_boot {
            hi.push(f("BOOTSZ", 0x06, "Boot section size (see boot loader table)"));
            hi.push(f("BOOTRST", 0x01, "Reset to the boot loader section when programmed (0)"));
        }
        let fuses = vec![
            low,
            FuseByteSpec { name: "High".into(), default: if has_boot { 0xd9 } else { 0xdf }, bits: hi },
            FuseByteSpec { name: "Extended".into(), default: 0xff, bits: vec![f("BODLEVEL", 0x07, "Brown-out detector level (111 disabled, 110 1.8 V, 101 2.7 V, 100 4.3 V)")] },
        ];

        // ----- groups -----
        let mut groups: Vec<(String, String)> = vec![("CPU".into(), "CPU, Clock, Sleep, Reset & Power".into())];
        groups.extend(pnames.iter().map(|l| (format!("PORT{l}"), format!("I/O Port {l}"))));
        groups.push(("EXINT".into(), "External & Pin Change Interrupts".into()));
        groups.extend(timers.iter().map(|&(n, wide)| (format!("TC{n}"), format!("{}-bit Timer/Counter{n} with PWM", if wide { 16 } else { 8 }))));
        groups.extend((0..usarts).map(|k| (format!("USART{k}"), format!("USART{k} (serial port)"))));
        groups.extend((0..spis).map(|k| (ix("SPI", k), format!("Serial Peripheral Interface {}", sx(k)).trim_end().to_string())));
        groups.extend((0..twis).map(|k| (ix("TWI", k), format!("2-wire Serial Interface (I2C) {}", sx(k)).trim_end().to_string())));
        if self.analog_comparator {
            groups.push(("AC".into(), "Analog Comparator".into()));
        }
        if adc > 0 {
            groups.push(("ADC".into(), "10-bit Analog to Digital Converter".into()));
        }
        if has_eeprom {
            groups.push(("EEPROM".into(), "EEPROM".into()));
        }
        groups.push(("WDT".into(), "Watchdog Timer".into()));

        // ----- core -----
        let mut features = feature::MOVW | feature::LPMX | feature::BREAK;
        if self.hardware_multiplier {
            features |= feature::MUL;
        }
        if has_boot {
            features |= feature::SPM;
        }
        if flash > 8192 {
            features |= feature::JMP;
        }
        if flash > 65536 {
            features |= feature::ELPM | feature::ELPMX | feature::SPMX;
        }
        if flash > 131072 {
            features |= feature::EIJMP;
        }
        let avr = match (flash > 131072, flash > 65536, flash > 8192) {
            (true, _, _) => "AVR6",
            (_, true, _) => "AVR51",
            (_, _, true) => "AVR5",
            _ => "AVR4",
        };
        let core_name = format!("{} ({avr}, custom)", if self.hardware_multiplier { "AVRe+" } else { "AVRe" });

        let boot_words = flash / 2;
        let boot = has_boot.then_some(BootSpec {
            sizes_words: if boot_words >= 32768 { [4096, 2048, 1024, 512] } else if boot_words >= 8192 { [2048, 1024, 512, 256] } else { [1024, 512, 256, 128] },
        });

        Ok(AvrDeviceSpec {
            id: self.id.clone(),
            name: self.name.trim().to_string(),
            family: "Custom".into(),
            core_name,
            features,
            flash_size: flash,
            sram_start: sram_start as u16,
            sram_size: self.sram_size as u16,
            eeprom_size: self.eeprom_size as u16,
            io_base: 0x20,
            io_size: 64,
            regs_in_data_space: true,
            flash_map_base: None,
            nvm_map: None,
            // Synthetic signature: manufacturer byte 0x1E, then 0xFF 0xFF (no such part exists).
            signature: [0x1e, 0xff, 0xff],
            calibration: 0x80,
            fuses,
            sleep: SleepControl {
                register: "SMCR".into(),
                se_mask: 0x01,
                sm_mask: 0x0e,
                modes: vec![
                    (0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave),
                    (6, SleepKind::Standby), (7, SleepKind::ExtendedStandby),
                ],
            },
            boot,
            vectors: vecs,
            registers: r,
            groups: groups.into_iter().map(|(name, desc)| PeripheralGroupSpec { name, desc }).collect(),
            package,
            pins,
            gpio_count: gpios as u8,
            has_adc: adc > 0,
            clock: ClockSpec { internal_hz: self.internal_hz, slow_hz: 128_000.0, default_prescale_log2: 3 },
            vcc: self.vcc,
            vcc_range: (1.8, 5.5),
            speed_grades: vec![(self.max_hz * 0.2, 1.8), (self.max_hz * 0.5, 2.7), (self.max_hz, 4.5)],
            datasheet: "User-defined device; register layout follows Microchip DS2549 (ATmega640/1280/1281/2560/2561)".into(),
            die: None,
            peripheral_set: PeripheralSet::Custom,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Structural invariants every generated spec must satisfy.
    pub fn check_spec(d: &AvrDeviceSpec) {
        let mut names: Vec<String> = d.registers.iter().map(|r| r.name.to_ascii_uppercase()).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "{}: duplicate register name", d.name);
        let mut addrs: Vec<u16> = d.registers.iter().map(|r| r.addr).collect();
        addrs.sort_unstable();
        addrs.dedup();
        assert_eq!(addrs.len(), d.registers.len(), "{}: duplicate register address", d.name);
        assert!(d.registers.iter().all(|r| r.addr < d.sram_start), "{}: register in SRAM", d.name);
        assert!(d.registers.iter().all(|r| r.addr >= 0x20), "{}: register below I/O base", d.name);
        assert!(d.gpio_names().iter().all(|n| !n.is_empty()), "{}: GPIO without a pin", d.name);
        let mut nums: Vec<u8> = d.pins.iter().map(|p| p.number).collect();
        nums.sort_unstable();
        nums.dedup();
        assert_eq!(nums.len(), d.pins.len(), "{}: duplicate pin number", d.name);
        let mut vn: Vec<&str> = d.vectors.iter().map(|v| v.name.as_str()).collect();
        vn.sort_unstable();
        vn.dedup();
        assert_eq!(vn.len(), d.vectors.len(), "{}: duplicate vector name", d.name);
        assert!(d.vectors.iter().enumerate().all(|(i, v)| v.index as usize == i));
        assert!(d.sram_start as u32 + d.sram_size as u32 <= 0xffff);
        assert!(d.registers.iter().any(|r| r.name == d.sleep.register));
        for r in &d.registers {
            for bit in &r.bits {
                assert_ne!(bit.mask, 0, "{}.{}", r.name, bit.name);
            }
        }
    }

    #[test]
    fn default_builds() {
        let d = CustomMcuConfig::default().build().unwrap();
        check_spec(&d);
        assert_eq!(d.sram_start, 0x200);
        assert_eq!(d.reg("PORTA"), 0x22);
        assert_eq!(d.reg("UDR0"), 0xc6);
        assert_eq!(d.reg("TCCR1A"), 0x80);
        assert_eq!(d.gpio_count, 24);
        assert_eq!(d.vector("USART0_RX"), Some(d.vector("SPI_STC").unwrap() + 1));
        assert!(d.register("RAMPZ").is_none() && d.register("EIND").is_none());
        assert!(d.features & feature::JMP != 0);
        assert_eq!(d.package, "PDIP-32");
    }

    #[test]
    fn tiny_and_huge_build() {
        let t = CustomMcuConfig::tiny().build().unwrap();
        check_spec(&t);
        assert_eq!(t.flash_size, 64);
        assert!(t.features & feature::JMP == 0);
        let h = CustomMcuConfig::huge().build().unwrap();
        check_spec(&h);
        assert_eq!(h.gpio_count, 248);
        assert!(h.register("EIND").is_some() && h.register("RAMPZ").is_some());
        assert_eq!(h.sram_start as u32 + h.sram_size as u32, 0xffff);
        assert_eq!(port_name(25), "AA");
        assert!(h.features & (feature::EIJMP | feature::ELPM | feature::ELPMX) == feature::EIJMP | feature::ELPM | feature::ELPMX);
    }

    #[test]
    fn validation() {
        let ok = CustomMcuConfig::default();
        let bad = |f: &dyn Fn(&mut CustomMcuConfig)| {
            let mut c = ok.clone();
            f(&mut c);
            c.validate().unwrap_err()
        };
        assert!(bad(&|c| c.flash_size = 63).contains("even"));
        assert!(bad(&|c| c.flash_size = 32).contains("64"));
        assert!(bad(&|c| c.flash_size = (8 << 20) + 2).contains("22 bits"));
        assert!(bad(&|c| c.sram_size = 16).contains("32"));
        assert!(bad(&|c| c.sram_size = 65400).contains("16-bit data space"));
        assert!(bad(&|c| c.eeprom_size = 65536).contains("EEAR"));
        assert!(bad(&|c| c.ports = 0).contains("Port"));
        assert!(bad(&|c| c.ports = 32).contains("255"));
        assert!(bad(&|c| c.adc_channels = 31).contains("MUX"));
        assert!(bad(&|c| c.id = "mychip".into()).contains("custom-"));
        assert!(bad(&|c| c.id = "custom-My Chip".into()).contains("custom-"));
        assert!(bad(&|c| c.package = "QFP".into()).contains("DIP"));
        assert!(bad(&|c| c.vcc = 9.0).contains("VCC"));
        // 128 B flash cannot hold the vector table of many peripherals.
        assert!(bad(&|c| {
            c.flash_size = 64;
            c.usarts = 8;
            c.timers16 = 8;
        })
        .contains("vector table"));
        // Odd, non power-of-two sizes are fine.
        let mut c = ok.clone();
        c.flash_size = 12346;
        c.sram_size = 777;
        c.eeprom_size = 100;
        c.validate().unwrap();
    }
}
