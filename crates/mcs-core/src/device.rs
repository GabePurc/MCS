//! Architecture-neutral device handle. Everything above the simulation cores (session, API,
//! UI protocol) deals with [`DeviceRef`]; only per-architecture code looks inside.

use serde::Serialize;

use crate::arm::device::ArmDeviceSpec;
use crate::avr::device::AvrDeviceSpec;

/// CPU architecture family of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    Avr,
    Arm,
}

/// Reference to a registered device of any architecture. Serializes as the architecture's spec
/// JSON plus an `"arch"` tag (`{"arch":"avr", "id":..., ...}`, `{"arch":"arm", ...}`).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "arch", rename_all = "lowercase")]
pub enum DeviceRef {
    Avr(&'static AvrDeviceSpec),
    Arm(&'static ArmDeviceSpec),
}

impl DeviceRef {
    pub fn id(&self) -> &'static str {
        match *self {
            DeviceRef::Avr(s) => s.id.as_str(),
            DeviceRef::Arm(s) => s.id.as_str(),
        }
    }

    pub fn name(&self) -> &'static str {
        match *self {
            DeviceRef::Avr(s) => s.name.as_str(),
            DeviceRef::Arm(s) => s.name.as_str(),
        }
    }

    pub fn arch(&self) -> Arch {
        match self {
            DeviceRef::Avr(_) => Arch::Avr,
            DeviceRef::Arm(_) => Arch::Arm,
        }
    }

    /// Program memory size in bytes.
    pub fn flash_size(&self) -> u32 {
        match *self {
            DeviceRef::Avr(s) => s.flash_size,
            DeviceRef::Arm(s) => s.flash_size,
        }
    }

    /// Address of the first byte of program memory (0 for AVR, 0x0800_0000 for STM32).
    pub fn flash_base(&self) -> u32 {
        match *self {
            DeviceRef::Avr(_) => 0,
            DeviceRef::Arm(s) => s.flash_base,
        }
    }

    /// The AVR spec, for AVR-only code paths (assembler, disassembler, definitions).
    pub fn as_avr(&self) -> Option<&'static AvrDeviceSpec> {
        match *self {
            DeviceRef::Avr(s) => Some(s),
            _ => None,
        }
    }

    /// The ARM spec, for ARM-only code paths.
    pub fn as_arm(&self) -> Option<&'static ArmDeviceSpec> {
        match *self {
            DeviceRef::Arm(s) => Some(s),
            _ => None,
        }
    }

    /// True when both refer to the same registered device.
    pub fn same_as(&self, other: &DeviceRef) -> bool {
        match (self, other) {
            (DeviceRef::Avr(a), DeviceRef::Avr(b)) => std::ptr::eq(*a, *b),
            (DeviceRef::Arm(a), DeviceRef::Arm(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }
}
