//! ARM device registry. Add new device families here.

mod stm32g4;
#[allow(dead_code)]
mod stm32g4_gen;

use std::sync::OnceLock;

use super::device::ArmDeviceSpec;

/// All ARM devices known to the simulator.
pub fn all() -> &'static [ArmDeviceSpec] {
    static DEVICES: OnceLock<Vec<ArmDeviceSpec>> = OnceLock::new();
    DEVICES.get_or_init(stm32g4::devices)
}

pub fn list() -> Vec<&'static ArmDeviceSpec> {
    all().iter().collect()
}

pub fn get(id: &str) -> Option<&'static ArmDeviceSpec> {
    all().iter().find(|d| d.id.eq_ignore_ascii_case(id))
}
