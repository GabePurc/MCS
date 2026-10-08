//! Device registry. Add new device specs here.

mod tiny_rc;

use super::device::AvrDeviceSpec;
use std::sync::OnceLock;

/// All AVR devices known to the simulator.
pub fn all() -> &'static [AvrDeviceSpec] {
    static DEVICES: OnceLock<Vec<AvrDeviceSpec>> = OnceLock::new();
    DEVICES.get_or_init(tiny_rc::devices)
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
    }
}
