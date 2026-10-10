//! Plain data types of the generated device tables (`stm32*_gen.rs`), shared by all STM32 families.

pub struct BitDef {
    pub name: &'static str,
    pub mask: u32,
    pub desc: &'static str,
}

pub struct RegDef {
    pub name: &'static str,
    pub off: u32,
    pub reset: u32,
    pub access: &'static str,
    pub desc: &'static str,
    pub bits: &'static [BitDef],
}

pub struct PinDef {
    pub number: u8,
    pub name: &'static str,
    /// 0 I/O, 1 supply, 2 ground, 3 reference.
    pub kind: u8,
    /// GPIO index (port * 16 + bit), -1 when the pin is not a GPIO.
    pub gpio: i16,
    pub functions: &'static [&'static str],
}
