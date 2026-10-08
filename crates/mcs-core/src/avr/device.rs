//! Declarative AVR device description. Everything the simulator, the assembler (`.include
//! "tn10def.inc"` is generated from this) and the UI (I/O view, pin diagram) need to know about
//! a part lives here, so supporting a new AVR is mostly a matter of writing a new spec.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct BitFieldSpec {
    pub name: String,
    /// Bit mask within the register. Multi-bit fields use contiguous masks.
    pub mask: u8,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegisterAccess {
    Rw,
    R,
    W,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IoRegisterSpec {
    pub name: String,
    /// Data-space address.
    pub addr: u16,
    pub reset: u8,
    /// Peripheral/group the register belongs to (for the I/O view tree).
    pub group: String,
    pub desc: String,
    pub bits: Vec<BitFieldSpec>,
    pub access: RegisterAccess,
}

#[derive(Clone, Debug, Serialize)]
pub struct VectorSpec {
    pub index: u8,
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PinKind {
    Io,
    Vcc,
    Gnd,
    /// Analog reference / analog supply (AREF, AVCC).
    Ref,
}

#[derive(Clone, Debug, Serialize)]
pub struct PinSpec {
    /// Physical package pin number.
    pub number: u8,
    pub name: String,
    pub kind: PinKind,
    /// Index into the machine's GPIO pin array (io pins only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpio: Option<u8>,
    pub functions: Vec<String>,
}

/// A fuse bit or multi-bit field (contiguous mask). Programmed = 0, like the datasheets.
#[derive(Clone, Debug, Serialize)]
pub struct FuseBitSpec {
    pub name: String,
    pub mask: u8,
    pub desc: String,
}

/// One fuse byte (tiny10: the single configuration byte; classic AVRs: low/high/extended).
#[derive(Clone, Debug, Serialize)]
pub struct FuseByteSpec {
    pub name: String,
    /// Factory (erased-chip) value.
    pub default: u8,
    pub bits: Vec<FuseBitSpec>,
}

/// Canonical sleep modes. `Machine` passes `kind as u8` to the peripherals.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SleepKind {
    Idle = 0,
    AdcNoiseReduction = 1,
    PowerDown = 2,
    PowerSave = 3,
    Standby = 4,
    ExtendedStandby = 5,
}

impl SleepKind {
    pub const COUNT: usize = 6;

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::AdcNoiseReduction => "ADC Noise Reduction",
            Self::PowerDown => "Power-down",
            Self::PowerSave => "Power-save",
            Self::Standby => "Standby",
            Self::ExtendedStandby => "Extended Standby",
        }
    }
}

/// Where SLEEP finds the enable bit and the mode bits.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepControl {
    pub register: String,
    pub se_mask: u8,
    /// Mode field (contiguous mask).
    pub sm_mask: u8,
    /// Mode field value -> canonical mode (values not listed are reserved).
    pub modes: Vec<(u8, SleepKind)>,
}

/// Boot loader section (classic ATmega): size in words per BOOTSZ field value.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootSpec {
    pub sizes_words: [u32; 4],
}

#[derive(Clone, Debug, Serialize)]
pub struct PeripheralGroupSpec {
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NvmMap {
    pub lock: u16,
    pub config: u16,
    pub calibration: u16,
    pub signature: u16,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSpec {
    pub internal_hz: f64,
    pub slow_hz: f64,
    pub default_prescale_log2: u8,
}

/// Physical silicon die (for the Device Info and Chip View windows).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DieSpec {
    pub width_um: f64,
    pub height_um: f64,
    /// Public die photograph and its credit/license.
    pub photo_url: String,
    pub photo_credit: String,
}

/// Peripheral wiring recipe used by the simulator's machine factory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PeripheralSet {
    /// ATtiny4/5/9/10.
    TinyRc,
    /// ATmega48/88/168/328 (P/PA).
    MegaX8,
    /// ATtiny25/45/85.
    TinyX5,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvrDeviceSpec {
    pub id: String,
    pub name: String,
    pub family: String,
    pub core_name: String,
    /// Feature mask, see `isa::feature`.
    pub features: u32,
    pub flash_size: u32,
    pub sram_start: u16,
    pub sram_size: u16,
    pub eeprom_size: u16,
    /// Data-space address of I/O address 0 (0x20 on classic cores, 0x00 on AVRrc).
    pub io_base: u16,
    /// Number of I/O addresses reachable with IN/OUT (64).
    pub io_size: u16,
    /// Whether r0..r31 are mapped at data addresses 0..31.
    pub regs_in_data_space: bool,
    /// Data-space address where flash is mapped for LD access (AVRrc: 0x4000).
    pub flash_map_base: Option<u16>,
    /// Non-volatile configuration areas mapped into data space (AVRrc).
    pub nvm_map: Option<NvmMap>,
    pub signature: [u8; 3],
    /// Factory oscillator calibration byte.
    pub calibration: u8,
    /// Fuse bytes in programming order (low, high, extended for classic AVRs).
    pub fuses: Vec<FuseByteSpec>,
    pub sleep: SleepControl,
    pub boot: Option<BootSpec>,
    pub vectors: Vec<VectorSpec>,
    pub registers: Vec<IoRegisterSpec>,
    pub groups: Vec<PeripheralGroupSpec>,
    pub package: String,
    pub pins: Vec<PinSpec>,
    /// Number of GPIO pins (indices 0..n of `PinSpec::gpio`).
    pub gpio_count: u8,
    pub has_adc: bool,
    pub clock: ClockSpec,
    /// Default supply voltage.
    pub vcc: f64,
    /// Operating voltage range (V).
    pub vcc_range: (f64, f64),
    /// Speed grades: (maximum clock in Hz, minimum VCC for it).
    pub speed_grades: Vec<(f64, f64)>,
    /// Data sheet the model follows (document name and revision).
    pub datasheet: String,
    pub die: Option<DieSpec>,
    pub peripheral_set: PeripheralSet,
}

impl AvrDeviceSpec {
    pub fn ram_end(&self) -> u16 {
        self.sram_start + self.sram_size - 1
    }

    pub fn flash_words(&self) -> u32 {
        self.flash_size / 2
    }

    /// Finds a register by name (case-insensitive).
    pub fn register(&self, name: &str) -> Option<&IoRegisterSpec> {
        self.registers.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }

    /// Data-space address of a register (panics if missing: wiring bug).
    pub fn reg(&self, name: &str) -> u16 {
        self.register(name).unwrap_or_else(|| panic!("{}: register {name} not defined", self.name)).addr
    }

    pub fn vector(&self, name: &str) -> Option<u8> {
        self.vectors.iter().find(|v| v.name == name).map(|v| v.index)
    }

    pub fn vector_count(&self) -> usize {
        self.vectors.iter().map(|v| v.index as usize + 1).max().unwrap_or(1)
    }

    /// Fuse field (by name) and its fuse byte index.
    pub fn fuse_field(&self, name: &str) -> Option<(usize, &FuseBitSpec)> {
        self.fuses.iter().enumerate().find_map(|(i, b)| b.bits.iter().find(|f| f.name == name).map(|f| (i, f)))
    }

    /// Default fuse bytes.
    pub fn fuse_defaults(&self) -> Vec<u8> {
        self.fuses.iter().map(|f| f.default).collect()
    }

    /// GPIO names indexed by GPIO number ("PB0", "PC6"...).
    pub fn gpio_names(&self) -> Vec<String> {
        let mut names = vec![String::new(); self.gpio_count as usize];
        for p in &self.pins {
            if let Some(g) = p.gpio {
                if let Some(n) = names.get_mut(g as usize) {
                    *n = p.name.clone();
                }
            }
        }
        names
    }

    /// Highest clock frequency allowed at `vcc` by the speed grades (0 when below all of them).
    pub fn max_hz_at(&self, vcc: f64) -> f64 {
        self.speed_grades.iter().filter(|g| vcc + 1e-9 >= g.1).map(|g| g.0).fold(0.0, f64::max)
    }

    /// Converts a data-space address to its I/O address when inside the IN/OUT range.
    pub fn data_to_io(&self, addr: u16) -> Option<u16> {
        addr.checked_sub(self.io_base).filter(|io| *io < self.io_size)
    }
}

/// Helper for register bit lists written MSB -> LSB with `None` for reserved bits.
pub fn bits_msb_first(names: &[Option<&str>], descs: &[(&str, &str)]) -> Vec<BitFieldSpec> {
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            n.map(|n| BitFieldSpec {
                name: n.to_string(),
                mask: 1 << (7 - i),
                desc: descs.iter().find(|d| d.0 == n).map(|d| d.1.to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

pub fn field(name: &str, mask: u8, desc: &str) -> BitFieldSpec {
    BitFieldSpec { name: name.into(), mask, desc: desc.into() }
}

/// Value of a (possibly multi-bit) fuse field from fuse bytes.
pub fn fuse_field_value(fuses: &[u8], byte: usize, f: &FuseBitSpec) -> u8 {
    (fuses.get(byte).copied().unwrap_or(0xff) & f.mask) >> f.mask.trailing_zeros()
}
