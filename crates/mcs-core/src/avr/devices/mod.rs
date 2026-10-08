//! Device registry. Add new device specs here.

mod mega_x8;
mod tiny_rc;
mod tiny_x5;

use super::device::AvrDeviceSpec;
use std::sync::OnceLock;

/// All AVR devices known to the simulator.
pub fn all() -> &'static [AvrDeviceSpec] {
    static DEVICES: OnceLock<Vec<AvrDeviceSpec>> = OnceLock::new();
    DEVICES.get_or_init(|| [tiny_rc::devices(), tiny_x5::devices(), mega_x8::devices()].concat())
}

pub fn get(id: &str) -> Option<&'static AvrDeviceSpec> {
    all().iter().find(|d| d.id.eq_ignore_ascii_case(id))
}

/// Maps an avrasm2 include name ("tn10def.inc") to a known device id ("attiny10").
pub fn id_from_include_name(name: &str) -> Option<&'static str> {
    let n = name.trim().to_ascii_lowercase();
    let stem = n.strip_suffix("def.inc")?;
    let (prefix, rest) = [("tn", "attiny"), ("m", "atmega"), ("usb", "at90usb"), ("can", "at90can"), ("pwm", "at90pwm"), ("x", "atxmega")]
        .iter()
        .find_map(|(p, full)| stem.strip_prefix(p).map(|r| (*full, r)))?;
    let id = format!("{prefix}{rest}");
    get(&id).map(|d| d.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // Every device: unique register addresses and names, pins cover all GPIOs.
        for d in all() {
            let mut addrs: Vec<u16> = d.registers.iter().map(|r| r.addr).collect();
            addrs.sort_unstable();
            addrs.dedup();
            assert_eq!(addrs.len(), d.registers.len(), "{}: duplicate register address", d.name);
            assert!(d.gpio_names().iter().all(|n| !n.is_empty()), "{}: GPIO without a pin", d.name);
            assert!(d.registers.iter().all(|r| r.addr < d.sram_start), "{}: register in SRAM", d.name);
        }
    }
}
