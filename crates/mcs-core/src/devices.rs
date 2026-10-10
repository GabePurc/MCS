//! Device registry across every architecture. Per-architecture registries (e.g.
//! [`crate::avr::devices`]) keep their typed lookups; this module is the neutral entry point.

use crate::arm::devices as arm;
use crate::avr::devices as avr;
use crate::riscv::devices as riscv;
use crate::device::DeviceRef;

/// Finds a device of any architecture by id (case-insensitive).
pub fn get_any(id: &str) -> Option<DeviceRef> {
    avr::get(id).map(DeviceRef::Avr).or_else(|| arm::get(id).map(DeviceRef::Arm)).or_else(|| riscv::get(id).map(DeviceRef::Riscv))
}

/// All devices: built-ins of every architecture followed by the registered custom devices.
pub fn list_any() -> Vec<DeviceRef> {
    avr::list().into_iter().map(DeviceRef::Avr).chain(arm::list().into_iter().map(DeviceRef::Arm)).chain(riscv::list().into_iter().map(DeviceRef::Riscv)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Arch;

    #[test]
    fn neutral_registry() {
        let d = get_any("ATtiny10").unwrap();
        assert_eq!((d.id(), d.arch(), d.flash_size()), ("attiny10", Arch::Avr, 1024));
        assert!(d.as_avr().is_some() && d.same_as(&get_any("attiny10").unwrap()));
        assert!(get_any("nope").is_none());
        assert_eq!(list_any().len(), avr::list().len() + arm::list().len() + riscv::list().len());
        let g4 = get_any("STM32G474RE").unwrap();
        assert_eq!((g4.arch(), g4.flash_size(), g4.as_avr().is_none()), (Arch::Arm, 512 * 1024, true));
        assert_eq!(serde_json::to_value(g4).unwrap()["arch"], "arm");
        let json = serde_json::to_value(d).unwrap();
        assert_eq!(json["arch"], "avr");
        assert_eq!(json["id"], "attiny10");
    }
}
