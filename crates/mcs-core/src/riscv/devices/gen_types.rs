//! Plain data types of the generated ESP32-C3 register tables (`esp32c3_gen.rs`).

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
