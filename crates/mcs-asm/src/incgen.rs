//! Generates avrasm2-style device definition includes (`tn10def.inc` ...) from a device spec.
//!
//! The same entry list drives both the include text shown to users ([`generate_def_include`])
//! and the symbol scope the assembler uses for `.include "tn10def.inc"` / the implicit device
//! include ([`def_include_symbols`]), so the two can never diverge.

use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use mcs_core::avr::device::AvrDeviceSpec;
use mcs_core::avr::devices;
use mcs_core::avr::isa::feature;

use crate::util::{FxMap, FxSet};

enum Entry {
    Section(&'static str),
    Comment(String),
    Equ { name: String, value: i64, digits: usize, note: String },
    Def { name: &'static str, reg: i64 },
}

/// Device-id prefixes and their avrasm2 include-file prefixes (longest first).
const INC_PREFIXES: [(&str, &str); 6] =
    [("atxmega", "x"), ("at90usb", "usb"), ("at90can", "can"), ("at90pwm", "pwm"), ("attiny", "tn"), ("atmega", "m")];

/// avrasm2 include name for a device: ATtiny10 -> "tn10def.inc", ATmega328P -> "m328Pdef.inc".
pub fn def_include_name(spec: &AvrDeviceSpec) -> String {
    if spec.id.starts_with("custom-") {
        // User-defined devices: "<id>def.inc" (matches `devices::id_from_include_name`).
        return format!("{}def.inc", spec.id);
    }
    let name = spec.name.as_str();
    for (prefix, short) in INC_PREFIXES {
        if name.get(..prefix.len()).is_some_and(|h| h.eq_ignore_ascii_case(prefix)) {
            return format!("{short}{}def.inc", &name[prefix.len()..]);
        }
    }
    format!("{name}def.inc")
}

/// `PREFIX<digits>SUFFIX` -> digits.
fn digits_between<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let d = s.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (!d.is_empty() && d.bytes().all(|c| c.is_ascii_digit())).then_some(d)
}

/// Atmel's legacy vector names (OVF0addr ...) for the datasheet names (TIM0_OVF, TIMER0_OVF...).
fn legacy_vector_name(name: &str) -> Option<String> {
    if let Some(rest) = name.strip_prefix("TIMER") {
        return legacy_vector_name(&format!("TIM{rest}"));
    }
    if let Some(n) = digits_between(name, "PCINT", "") {
        return Some(format!("PCI{n}"));
    }
    if let Some(n) = digits_between(name, "TIM", "_OVF") {
        return Some(format!("OVF{n}"));
    }
    if let Some(n) = digits_between(name, "TIM", "_CAPT") {
        return Some(format!("ICP{n}"));
    }
    if let Some(rest) = name.strip_prefix("TIM") {
        if let Some(p) = rest.find("_COMP") {
            let (n, tail) = (&rest[..p], &rest[p + 5..]);
            let tb = tail.as_bytes();
            if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) && tb.len() == 1 && tb[0].is_ascii_uppercase() {
                return Some(format!("OC{n}{tail}"));
            }
        }
    }
    match name {
        "ANA_COMP" | "ANALOG_COMP" => Some("ACI".into()),
        "ADC" => Some("ADCC".into()),
        "EE_RDY" | "EE_READY" => Some("ERDY".into()),
        "SPI_STC" => Some("SPI".into()),
        "USART_RX" => Some("URXC".into()),
        "USART_UDRE" => Some("UDRE".into()),
        "USART_TX" => Some("UTXC".into()),
        "SPM_READY" | "SPM_RDY" => Some("SPMR".into()),
        _ => None,
    }
}

struct Builder {
    out: Vec<Entry>,
    seen: FxSet<String>,
}

impl Builder {
    fn equ(&mut self, name: String, value: i64, digits: usize, note: String) {
        if self.seen.insert(name.to_ascii_uppercase()) {
            self.out.push(Entry::Equ { name, value, digits, note });
        }
    }
}

fn build_entries(spec: &AvrDeviceSpec) -> Vec<Entry> {
    let mut b = Builder { out: Vec::with_capacity(160), seen: FxSet::default() };

    b.out.push(Entry::Section("SIGNATURE"));
    for (i, &s) in spec.signature.iter().enumerate() {
        b.equ(format!("SIGNATURE_00{i}"), s as i64, 2, String::new());
    }

    b.out.push(Entry::Section("I/O REGISTER DEFINITIONS"));
    b.out.push(Entry::Comment(
        "Values are I/O addresses (IN/OUT/SBI/CBI/SBIC/SBIS); \"MEMORY MAPPED\" registers use data addresses (LDS/STS only)."
            .into(),
    ));
    let mut by_addr_desc: Vec<_> = spec.registers.iter().collect();
    by_addr_desc.sort_by_key(|r| std::cmp::Reverse(r.addr));
    for r in &by_addr_desc {
        let io = r.addr as i64 - spec.io_base as i64;
        let in_io = io >= 0 && io < spec.io_size as i64;
        if in_io {
            b.equ(r.name.clone(), io, 2, r.desc.clone());
        } else {
            b.equ(r.name.clone(), r.addr as i64, 4, format!("MEMORY MAPPED: {}", r.desc));
        }
    }

    b.out.push(Entry::Section("BIT DEFINITIONS"));
    let mut by_addr: Vec<_> = spec.registers.iter().collect();
    by_addr.sort_by_key(|r| r.addr);
    for r in &by_addr {
        if r.bits.is_empty() {
            continue;
        }
        b.out.push(Entry::Comment(format!("{} - {}", r.name, r.desc)));
        let sreg = r.name == "SREG";
        // `PORTB` -> Some("B")
        let port = r.name.strip_prefix("PORT").filter(|l| l.len() == 1 && l.as_bytes()[0].is_ascii_uppercase());
        for bit in &r.bits {
            let positions: Vec<i64> = (0..8).filter(|p| bit.mask & (1 << p) != 0).collect();
            if positions.len() == 1 {
                let pos = positions[0];
                let name = if sreg { format!("SREG_{}", bit.name) } else { bit.name.clone() };
                b.equ(name, pos, 0, bit.desc.clone());
                if let Some(letter) = port {
                    if let Some(n) = digits_between(&bit.name, &format!("PORT{letter}"), "") {
                        b.equ(format!("P{letter}{n}"), pos, 0, "For compatibility".into());
                    }
                }
            } else {
                // Multi-bit field: CS0 (mask 0x07) -> CS00, CS01, CS02; WDP -> WDP0..WDP2.
                let what = if bit.desc.is_empty() { bit.name.as_str() } else { bit.desc.as_str() };
                for (k, &pos) in positions.iter().enumerate() {
                    b.equ(format!("{}{k}", bit.name), pos, 0, format!("{what} bit {k}"));
                }
            }
        }
    }

    b.out.push(Entry::Section("CPU REGISTER DEFINITIONS"));
    for (name, reg) in [("XH", 27), ("XL", 26), ("YH", 29), ("YL", 28), ("ZH", 31), ("ZL", 30)] {
        b.out.push(Entry::Def { name, reg });
    }

    b.out.push(Entry::Section("DATA MEMORY DECLARATIONS"));
    let ram_end = spec.sram_start as i64 + spec.sram_size as i64 - 1;
    let e2end = (spec.eeprom_size as i64 - 1).max(0);
    let none = String::new;
    b.equ("FLASHEND".into(), (spec.flash_size >> 1) as i64 - 1, 4, "Note: Word address".into());
    b.equ("IOEND".into(), spec.io_size as i64 - 1, 4, none());
    b.equ("SRAM_START".into(), spec.sram_start as i64, 4, none());
    b.equ("RAMSTART".into(), spec.sram_start as i64, 4, none());
    b.equ("SRAM_SIZE".into(), spec.sram_size as i64, 0, none());
    b.equ("RAMEND".into(), ram_end, 4, none());
    b.equ("XRAMEND".into(), 0, 4, none());
    b.equ("E2END".into(), e2end, 4, none());
    b.equ("EEPROMEND".into(), e2end, 4, none());
    let ee = spec.eeprom_size as u32;
    let eeadrbits = if ee > 1 { 32 - (ee - 1).leading_zeros() } else { 0 };
    b.equ("EEADRBITS".into(), eeadrbits as i64, 0, none());
    if let Some(base) = spec.flash_map_base {
        b.equ("MAPPED_FLASH_START".into(), base as i64, 4, none());
        b.equ("MAPPED_FLASH_END".into(), base as i64 + spec.flash_size as i64 - 1, 4, none());
    }

    b.out.push(Entry::Section("INTERRUPT VECTORS"));
    let vec_words: i64 = if spec.features & feature::JMP != 0 { 2 } else { 1 };
    let mut max_index = 0i64;
    for v in &spec.vectors {
        let index = v.index as i64;
        max_index = max_index.max(index);
        if index == 0 {
            continue; // RESET is always word 0
        }
        b.equ(format!("{}addr", v.name), index * vec_words, 4, v.desc.clone());
        if let Some(legacy) = legacy_vector_name(&v.name) {
            b.equ(format!("{legacy}addr"), index * vec_words, 4, format!("{} (Atmel name)", v.desc));
        }
    }
    b.equ("INT_VECTORS_SIZE".into(), (max_index + 1) * vec_words, 0, "size in words".into());
    b.out
}

fn fmt_value(out: &mut String, value: i64, digits: usize) {
    if digits == 0 {
        let _ = write!(out, "{value}");
    } else {
        let hex = if value < 0 { format!("-{:x}", value.unsigned_abs()) } else { format!("{value:x}") };
        let _ = write!(out, "0x{hex:0>digits$}");
    }
}

/// avrasm2-compatible definitions file for `spec` (equivalent to Atmel's `tn10def.inc`).
pub fn generate_def_include(spec: &AvrDeviceSpec) -> String {
    let name = def_include_name(spec);
    let mut out = String::with_capacity(8192);
    let _ = write!(
        out,
        ";***** Generated by MCS from the device description - DO NOT EDIT *****\n\
         ;* File Name         : \"{name}\"\n\
         ;* Title             : Register/Bit Definitions for the {}\n\
         ;* Core              : {}\n\
         ;* Flash/SRAM/EEPROM : {} / {} / {} bytes\n\
         ;*************************************************************************\n\
         \n\
         ; ***** SPECIFY DEVICE ***************************************************\n\
         .device {}",
        spec.name, spec.core_name, spec.flash_size, spec.sram_size, spec.eeprom_size,
        if spec.id.starts_with("custom-") { spec.id.as_str() } else { spec.name.as_str() }
    );
    for e in build_entries(spec) {
        match e {
            Entry::Section(title) => {
                let stars = 64usize.saturating_sub(title.len()).max(3);
                let _ = write!(out, "\n\n; ***** {title} {}", "*".repeat(stars));
            }
            Entry::Comment(text) => {
                let _ = write!(out, "\n; {text}");
            }
            Entry::Equ { name, value, digits, note } => {
                let _ = write!(out, "\n.equ\t{name}\t= ");
                fmt_value(&mut out, value, digits);
                if !note.is_empty() {
                    let _ = write!(out, "\t; {note}");
                }
            }
            Entry::Def { name, reg } => {
                let _ = write!(out, "\n.def\t{name}\t= r{reg}");
            }
        }
    }
    out.push('\n');
    out
}

/// Symbols defined by the generated include for a device.
#[derive(Debug, Default)]
pub(crate) struct DefSymbols {
    /// Constants by lowercase name.
    pub equs: FxMap<String, i64>,
    /// Register aliases (XL..ZH) by lowercase name.
    pub defs: FxMap<String, i64>,
}

fn compute_symbols(spec: &AvrDeviceSpec) -> DefSymbols {
    let mut syms = DefSymbols::default();
    for e in build_entries(spec) {
        match e {
            Entry::Equ { name, value, .. } => {
                syms.equs.insert(name.to_ascii_lowercase(), value);
            }
            Entry::Def { name, reg } => {
                syms.defs.insert(name.to_ascii_lowercase(), reg);
            }
            _ => {}
        }
    }
    syms
}

/// Symbols defined by the generated include for `spec` (cached for registry devices).
pub(crate) fn def_include_symbols(spec: &AvrDeviceSpec) -> Arc<DefSymbols> {
    static CACHE: OnceLock<Vec<OnceLock<Arc<DefSymbols>>>> = OnceLock::new();
    let all = devices::all();
    match all.iter().position(|d| std::ptr::eq(d, spec)) {
        Some(i) => {
            let cache = CACHE.get_or_init(|| all.iter().map(|_| OnceLock::new()).collect());
            match cache.get(i) {
                Some(slot) => slot.get_or_init(|| Arc::new(compute_symbols(spec))).clone(),
                None => Arc::new(compute_symbols(spec)),
            }
        }
        None => Arc::new(compute_symbols(spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names() {
        assert_eq!(legacy_vector_name("PCINT0").as_deref(), Some("PCI0"));
        assert_eq!(legacy_vector_name("TIM0_OVF").as_deref(), Some("OVF0"));
        assert_eq!(legacy_vector_name("TIM0_COMPB").as_deref(), Some("OC0B"));
        assert_eq!(legacy_vector_name("TIM1_CAPT").as_deref(), Some("ICP1"));
        assert_eq!(legacy_vector_name("TIM_OVF"), None);
        assert_eq!(legacy_vector_name("TIM0_COMPb"), None);
        assert_eq!(legacy_vector_name("ADC").as_deref(), Some("ADCC"));
        assert_eq!(legacy_vector_name("INT0"), None);
        assert_eq!(legacy_vector_name("TIMER1_COMPA").as_deref(), Some("OC1A"));
        assert_eq!(legacy_vector_name("TIMER2_OVF").as_deref(), Some("OVF2"));
        assert_eq!(legacy_vector_name("USART_RX").as_deref(), Some("URXC"));
        assert_eq!(legacy_vector_name("EE_READY").as_deref(), Some("ERDY"));
    }

    #[test]
    fn cached_symbols() {
        let t10 = devices::get("attiny10").unwrap();
        let a = def_include_symbols(t10);
        let b = def_include_symbols(t10);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.equs.get("portb"), Some(&2));
        assert_eq!(a.defs.get("zl"), Some(&30));
    }
}
