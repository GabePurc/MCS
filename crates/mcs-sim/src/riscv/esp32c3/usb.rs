//! USB Serial/JTAG controller (ESP32-C3 TRM "USB Serial/JTAG Controller"), CDC-ACM serial endpoint only.
//!
//! The host side is assumed to be connected and to drain the endpoint immediately: bytes written to
//! `EP1` (`RDWR_BYTE`) appear in the Serial Monitor at once and the IN FIFO is always free
//! (`EP1_CONF.SERIAL_IN_EP_DATA_FREE` reads 1). Every written byte (and `EP1_CONF.WR_DONE`) re-arms
//! `SERIAL_IN_EMPTY_INT_RAW`. Interrupt matrix source 26.
//!
//! Not modelled: the host-to-device direction (the Serial Monitor input goes to UART0), the JTAG bridge,
//! USB bus events and the SOF counter.

use mcs_core::riscv::device::RiscvDeviceSpec;

use crate::riscv::bus::{Cx, Mmio};

use super::misc::RegFile;

const EP1: u32 = 0x00;
const EP1_CONF: u32 = 0x04;
const INT_RAW: u32 = 0x08;
const INT_ST: u32 = 0x0c;
const INT_ENA: u32 = 0x10;
const INT_CLR: u32 = 0x14;
const JFIFO_ST: u32 = 0x20;

const SERIAL_IN_EMPTY: u32 = 1 << 3;
const SRC_USB: u8 = 26;

pub struct UsbSerial {
    regs: RegFile,
    int_raw: u32,
    int_ena: u32,
}

impl UsbSerial {
    pub fn new(spec: &RiscvDeviceSpec) -> Self {
        Self { regs: RegFile::from_spec(spec, "USB_DEVICE", spec.peripheral_set.usb_base, 0x100), int_raw: SERIAL_IN_EMPTY, int_ena: 0 }
    }

    fn irq(&self, cx: &mut Cx) {
        cx.irq_source(SRC_USB, self.int_raw & self.int_ena & 0xfff != 0);
    }
}

impl Mmio for UsbSerial {
    fn read(&mut self, off: u32, _size: u8, _cx: &mut Cx) -> u32 {
        match off {
            EP1 => 0,
            EP1_CONF => 2,
            INT_RAW => self.int_raw,
            INT_ST => self.int_raw & self.int_ena,
            INT_ENA => self.int_ena,
            // IN FIFO empty, OUT FIFO empty.
            JFIFO_ST => 0x44,
            _ => self.regs.get(off),
        }
    }

    fn write(&mut self, off: u32, _size: u8, v: u32, cx: &mut Cx) {
        match off {
            EP1 => {
                cx.sys.serial_out.push(v as u8);
                self.int_raw |= SERIAL_IN_EMPTY;
                self.irq(cx);
            }
            EP1_CONF => {
                if v & 1 != 0 {
                    self.int_raw |= SERIAL_IN_EMPTY;
                    self.irq(cx);
                }
            }
            INT_RAW => {
                self.int_raw |= v & 0xfff;
                self.irq(cx);
            }
            INT_ENA => {
                self.int_ena = v & 0xfff;
                self.irq(cx);
            }
            INT_CLR => {
                self.int_raw &= !(v & 0xfff);
                self.irq(cx);
            }
            INT_ST | JFIFO_ST => {}
            _ => self.regs.put(off, v),
        }
    }

    fn peek(&mut self, off: u32, cx: &mut Cx) -> u32 {
        self.read(off & !3, 4, cx)
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.regs.reset();
        self.int_raw = SERIAL_IN_EMPTY;
        self.int_ena = 0;
        self.irq(cx);
    }
}
