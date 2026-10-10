//! RISC-V device registry. Add new device families here.

#[allow(dead_code)]
mod esp32c3_gen;
#[allow(dead_code)]
mod gen_types;
mod esp32c3;

use std::sync::OnceLock;

use super::device::RiscvDeviceSpec;

/// All RISC-V devices known to the simulator.
pub fn all() -> &'static [RiscvDeviceSpec] {
    static DEVICES: OnceLock<Vec<RiscvDeviceSpec>> = OnceLock::new();
    DEVICES.get_or_init(esp32c3::devices)
}

pub fn list() -> Vec<&'static RiscvDeviceSpec> {
    all().iter().collect()
}

pub fn get(id: &str) -> Option<&'static RiscvDeviceSpec> {
    all().iter().find(|d| d.id.eq_ignore_ascii_case(id))
}
