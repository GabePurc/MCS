//! Architecture-neutral device handle. Everything above the simulation cores (session, API,
//! UI protocol) deals with [`DeviceRef`]; only per-architecture code looks inside.

use serde::Serialize;

use crate::avr::device::AvrDeviceSpec;

/// CPU architecture family of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    Avr,
}

/// Reference to a registered device of any architecture. Serializes as the architecture's spec
/// JSON plus an `"arch"` tag (`{"arch":"avr", "id":..., ...}`).
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "arch", rename_all = "lowercase")]
pub enum DeviceRef {
    Avr(&'static AvrDeviceSpec),
}

impl DeviceRef {
    pub fn id(&self) -> &'static str {
        match *self {
            DeviceRef::Avr(s) => s.id.as_str(),
        }
    }

    pub fn name(&self) -> &'static str {
        match *self {
            DeviceRef::Avr(s) => s.name.as_str(),
        }
    }

    pub fn arch(&self) -> Arch {
        match self {
            DeviceRef::Avr(_) => Arch::Avr,
        }
    }

    /// Program memory size in bytes.
    pub fn flash_size(&self) -> u32 {
        match *self {
            DeviceRef::Avr(s) => s.flash_size,
        }
    }

    /// The AVR spec, for AVR-only code paths (assembler, disassembler, definitions).
    pub fn as_avr(&self) -> Option<&'static AvrDeviceSpec> {
        match *self {
            DeviceRef::Avr(s) => Some(s),
        }
    }

    /// True when both refer to the same registered device.
    pub fn same_as(&self, other: &DeviceRef) -> bool {
        match (self, other) {
            (DeviceRef::Avr(a), DeviceRef::Avr(b)) => std::ptr::eq(*a, *b),
        }
    }
}
