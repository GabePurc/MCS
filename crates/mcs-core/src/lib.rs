//! Shared foundation of the MCS microcontroller simulator.
//!
//! * [`program`] — architecture-neutral program images + debug info (symbols, line table).
//! * [`avr`] — AVR instruction set table and declarative device descriptions.
//!
//! * [`device`] / [`devices`] — architecture-neutral device handle and registry (`DeviceRef`, `get_any`).
//!
//! Execution (`mcs-sim`), the assembler (`mcs-asm`) and file loaders (`mcs-formats`) build on
//! these types. A future architecture adds a sibling module (e.g. `arm`) next to `avr`.

pub mod avr;
pub mod device;
pub mod devices;
pub mod program;
