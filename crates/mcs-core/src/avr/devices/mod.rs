//! Device registry. Add new device specs here.

mod custom;
mod mega_big;
mod mega_legacy;
mod mega_x0;
mod mega_x4;
mod mega_x8;
mod tiny_13;
mod tiny_rc;
mod tiny_x313;
mod tiny_x4;
mod tiny_x5;

use super::device::AvrDeviceSpec;
use std::sync::{OnceLock, RwLock};

pub use custom::{didr_name, port_name, timer_numbers, CustomMcuConfig};

/// All AVR devices known to the simulator.
pub fn all() -> &'static [AvrDeviceSpec] {
    static DEVICES: OnceLock<Vec<AvrDeviceSpec>> = OnceLock::new();
    DEVICES.get_or_init(|| [tiny_rc::devices(), tiny_x5::devices(), tiny_13::devices(), tiny_x4::devices(), tiny_x313::devices(), mega_x8::devices(), mega_legacy::devices(), mega_x4::devices(), mega_x0::devices()].concat())
}

static CUSTOM: RwLock<Vec<&'static AvrDeviceSpec>> = RwLock::new(Vec::new());

/// Registers (or replaces, by id) a user-defined device and returns its spec. Specs are leaked
/// (`'static` is required by machines); replacing an id leaks the previous spec, which is
/// acceptable because a spec is a few KB and replacements are user-driven.
pub fn register_custom(cfg: &CustomMcuConfig) -> Result<&'static AvrDeviceSpec, String> {
    let id = cfg.id.to_ascii_lowercase();
    if all().iter().any(|d| d.id.eq_ignore_ascii_case(&id)) {
        return Err(format!("Device id '{id}' is a built-in device"));
    }
    let mut cfg = cfg.clone();
    cfg.id = id;
    let spec: &'static AvrDeviceSpec = Box::leak(Box::new(cfg.build()?));
    let mut list = CUSTOM.write().unwrap_or_else(|e| e.into_inner());
    match list.iter_mut().find(|d| d.id == spec.id) {
        Some(slot) => *slot = spec,
        None => list.push(spec),
    }
    Ok(spec)
}

/// Built-in devices followed by the registered custom devices.
pub fn list() -> Vec<&'static AvrDeviceSpec> {
    let mut v: Vec<&'static AvrDeviceSpec> = all().iter().collect();
    v.extend(CUSTOM.read().unwrap_or_else(|e| e.into_inner()).iter().copied());
    v
}

pub fn get(id: &str) -> Option<&'static AvrDeviceSpec> {
    all()
        .iter()
        .find(|d| d.id.eq_ignore_ascii_case(id))
        .or_else(|| CUSTOM.read().unwrap_or_else(|e| e.into_inner()).iter().copied().find(|d| d.id.eq_ignore_ascii_case(id)))
}

/// Maps an avrasm2 include name ("tn10def.inc") to a known device id ("attiny10").
pub fn id_from_include_name(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    let stem = n.strip_suffix("def.inc")?;
    if stem.starts_with("custom-") {
        return get(stem).map(|d| d.id.as_str());
    }
    let (prefix, rest) = [("tn", "attiny"), ("m", "atmega"), ("usb", "at90usb"), ("can", "at90can"), ("pwm", "at90pwm"), ("x", "atxmega")]
        .iter()
        .find_map(|(p, full)| stem.strip_prefix(p).map(|r| (*full, r)))?;
    let id = format!("{prefix}{rest}");
    get(&id).map(|d| d.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avr::device::PeripheralSet;

    #[test]
    fn registry() {
        let t10 = get("ATtiny10").unwrap();
        assert_eq!(t10.flash_size, 1024);
        assert_eq!(t10.reg("PORTB"), 0x02);
        assert_eq!(t10.vector("TIM0_OVF"), Some(4));
        assert_eq!(t10.vector_count(), 11);
        assert_eq!(id_from_include_name("tn10def.inc"), Some("attiny10"));
        assert_eq!(get("attiny9").unwrap().vector_count(), 10);
        assert_eq!(t10.max_hz_at(5.0), 12e6);
        assert_eq!(t10.max_hz_at(3.3), 8e6);
        assert_eq!(t10.max_hz_at(1.5), 0.0);
        let m = get("ATmega328P").unwrap();
        assert_eq!(m.reg("UDR0"), 0xc6);
        assert_eq!(m.vector("USART_RX"), Some(18));
        assert_eq!(m.vector_count(), 26);
        assert_eq!(m.fuse_defaults(), [0x62, 0xd9, 0xff]);
        assert_eq!(m.gpio_names()[14], "PC6");
        assert_eq!(id_from_include_name("m328Pdef.inc"), Some("atmega328p"));
        let t = get("attiny85").unwrap();
        assert_eq!(t.reg("PORTB"), 0x38);
        assert_eq!(t.vector("USI_OVF"), Some(14));
        assert_eq!(id_from_include_name("tn85def.inc"), Some("attiny85"));
        let t13 = get("ATtiny13A").unwrap();
        assert_eq!((t13.flash_size, t13.sram_size, t13.eeprom_size, t13.signature), (1024, 64, 64, [0x1e, 0x90, 0x07]));
        assert_eq!(t13.reg("TIFR0"), 0x58);
        assert_eq!(t13.vector("TIM0_COMPB"), Some(7));
        assert_eq!(t13.vector_count(), 10);
        assert_eq!(t13.fuse_defaults(), [0x6a, 0xff]);
        assert_eq!(id_from_include_name("tn13Adef.inc"), Some("attiny13a"));
        let t84 = get("attiny84a").unwrap();
        assert_eq!((t84.flash_size, t84.sram_size, t84.signature), (8192, 512, [0x1e, 0x93, 0x0c]));
        assert_eq!(t84.reg("TCNT1L"), 0x4c);
        assert_eq!(t84.reg("SPH"), 0x5e);
        assert_eq!(t84.vector("USI_OVF"), Some(16));
        assert_eq!(t84.vector_count(), 17);
        assert_eq!(t84.gpio_names()[11], "PB3");
        assert_eq!(get("ATtiny44A").unwrap().signature, [0x1e, 0x92, 0x07]);
        assert_eq!(id_from_include_name("tn24Adef.inc"), Some("attiny24a"));
        let t2313 = get("attiny2313a").unwrap();
        assert_eq!((t2313.flash_size, t2313.sram_size, t2313.signature), (2048, 128, [0x1e, 0x91, 0x0a]));
        assert_eq!(t2313.reg("UDR"), 0x2c);
        assert_eq!(t2313.vector("PCINT2"), Some(20));
        assert_eq!(t2313.vector_count(), 21);
        assert_eq!(t2313.gpio_names()[17], "PD6");
        assert_eq!(get("attiny4313").unwrap().signature, [0x1e, 0x92, 0x0d]);
        assert_eq!(id_from_include_name("tn2313Adef.inc"), Some("attiny2313a"));
        assert_eq!(id_from_include_name("tn4313def.inc"), Some("attiny4313"));
        let m8 = get("ATmega8").unwrap();
        assert_eq!((m8.flash_size, m8.sram_size, m8.eeprom_size, m8.signature), (8192, 1024, 512, [0x1e, 0x93, 0x07]));
        assert_eq!((m8.reg("UBRRH"), m8.reg("UCSRC")), (0x40, 0x40), "shared address");
        assert_eq!(m8.reg("TCCR0"), 0x53);
        assert_eq!(m8.vector("TWI"), Some(17));
        assert_eq!(m8.vector_count(), 19);
        assert_eq!(m8.fuse_defaults(), [0xe1, 0xd9]);
        assert_eq!(m8.gpio_names()[14], "PC6");
        assert_eq!(id_from_include_name("m8def.inc"), Some("atmega8"));
        let m16 = get("ATmega16").unwrap();
        assert_eq!((m16.flash_size, m16.signature, m16.reg("OCR0")), (16384, [0x1e, 0x94, 0x03], 0x5c));
        assert_eq!((m16.vector("INT2"), m16.vector("TIMER0_COMP"), m16.vector_count()), (Some(18), Some(19), 21));
        assert_eq!(m16.fuse_defaults(), [0xe1, 0x99]);
        assert_eq!((m16.sleep.se_mask, m16.sleep.sm_mask), (0x40, 0xb0));
        assert_eq!(m16.gpio_names()[31], "PD7");
        assert_eq!(id_from_include_name("m16def.inc"), Some("atmega16"));
        let m32 = get("ATmega32").unwrap();
        assert_eq!((m32.flash_size, m32.sram_size, m32.eeprom_size, m32.signature), (32768, 2048, 1024, [0x1e, 0x95, 0x02]));
        assert_eq!((m32.vector("INT2"), m32.vector("TIMER0_COMP"), m32.vector("TIMER0_OVF"), m32.vector_count()), (Some(3), Some(10), Some(11), 21));
        assert_eq!(m32.boot.as_ref().unwrap().sizes_words, [2048, 1024, 512, 256]);
        assert_eq!(id_from_include_name("m32def.inc"), Some("atmega32"));
        let m164 = get("ATmega164PA").unwrap();
        assert_eq!((m164.flash_size, m164.sram_size, m164.eeprom_size, m164.signature), (16384, 1024, 512, [0x1e, 0x94, 0x0a]));
        assert_eq!((m164.reg("UDR1"), m164.reg("PCMSK3"), m164.reg("PRR0"), m164.reg("TCCR1A")), (0xce, 0x73, 0x64, 0x80));
        assert_eq!((m164.vector("USART1_TX"), m164.vector("PCINT3"), m164.vector_count()), (Some(30), Some(7), 31));
        assert!(m164.register("TCCR3A").is_none() && m164.register("PRR1").is_none() && m164.register("RAMPZ").is_none());
        assert_eq!((m164.package.as_str(), m164.pins.len(), m164.gpio_count), ("PDIP-40", 40, 32));
        assert_eq!(m164.fuse_defaults(), [0x62, 0x99, 0xff]);
        assert_eq!(id_from_include_name("m164PAdef.inc"), Some("atmega164pa"));
        assert_eq!(get("atmega324pa").unwrap().signature, [0x1e, 0x95, 0x11]);
        let m644 = get("atmega644pa").unwrap();
        assert_eq!((m644.flash_size, m644.sram_size, m644.eeprom_size, m644.vector_count()), (65536, 4096, 2048, 31));
        assert_eq!(m644.boot.as_ref().unwrap().sizes_words, [4096, 2048, 1024, 512]);
        let m1284 = get("ATmega1284P").unwrap();
        assert_eq!((m1284.flash_size, m1284.sram_size, m1284.eeprom_size, m1284.signature), (131072, 16384, 4096, [0x1e, 0x97, 0x05]));
        assert_eq!((m1284.vector("TIMER3_OVF"), m1284.vector_count(), m1284.reg("TCCR3A"), m1284.reg("TIMSK3"), m1284.reg("PRR1"), m1284.reg("RAMPZ")), (Some(34), 35, 0x90, 0x71, 0x65, 0x5b));
        assert_eq!(m1284.ram_end(), 0x40ff);
        assert_eq!(id_from_include_name("m1284Pdef.inc"), Some("atmega1284p"));
        let m2560 = get("ATmega2560").unwrap();
        assert_eq!((m2560.flash_size, m2560.sram_size, m2560.eeprom_size, m2560.signature), (262144, 8192, 4096, [0x1e, 0x98, 0x01]));
        assert_eq!((m2560.package.as_str(), m2560.pins.len(), m2560.gpio_count, m2560.vector_count()), ("TQFP-100", 100, 86, 57));
        assert_eq!((m2560.reg("PORTL"), m2560.reg("UDR3"), m2560.reg("TCCR5A"), m2560.reg("OCR5CL"), m2560.reg("EIND"), m2560.sram_start), (0x10b, 0x136, 0x120, 0x12c, 0x5c, 0x200));
        assert_eq!((m2560.vector("TIMER1_COMPC"), m2560.vector("INT7"), m2560.vector("USART3_TX")), (Some(19), Some(8), Some(56)));
        assert_eq!(m2560.fuse_defaults(), [0x62, 0x99, 0xff]);
        assert_eq!(m2560.gpio_names()[85], "PL7");
        assert!(get("atmega1280").unwrap().register("EIND").is_none() && get("atmega640").unwrap().register("RAMPZ").is_some());
        assert_eq!(get("atmega640").unwrap().signature, [0x1e, 0x96, 0x08]);
        assert_eq!(id_from_include_name("m2560def.inc"), Some("atmega2560"));
        // Every device: unique register addresses and names, pins cover all GPIOs.
        let customs: Vec<_> = [CustomMcuConfig::default(), CustomMcuConfig::tiny(), CustomMcuConfig::huge()]
            .iter()
            .map(|c| register_custom(c).unwrap())
            .collect();
        assert_eq!(get("CUSTOM-tiny").unwrap().id, "custom-tiny");
        assert_eq!(id_from_include_name("custom-tinydef.inc"), Some("custom-tiny"));
        assert!(register_custom(&CustomMcuConfig { id: "custom-bad".into(), ports: 0, ..Default::default() }).is_err());
        assert!(list().len() >= all().len() + 3);
        for d in all().iter().chain(customs) {
            // ATmega8/16/32: UBRRH and UCSRC share one address (URSEL selects the register); that
            // pair is the only allowed exception.
            let mut addrs: Vec<u16> = d.registers.iter().filter(|r| !(r.name == "UCSRC" && d.peripheral_set == PeripheralSet::MegaLegacy)).map(|r| r.addr).collect();
            addrs.sort_unstable();
            addrs.dedup();
            let shared = if d.peripheral_set == PeripheralSet::MegaLegacy { 1 } else { 0 };
            assert_eq!(addrs.len() + shared, d.registers.len(), "{}: duplicate register address", d.name);
            assert!(d.gpio_names().iter().all(|n| !n.is_empty()), "{}: GPIO without a pin", d.name);
            assert!(d.registers.iter().all(|r| r.addr < d.sram_start), "{}: register in SRAM", d.name);
        }
    }
}
