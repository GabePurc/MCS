//! The ESP32-C3 microcontroller: the RV32IMC [`Machine`] with its memory map and the peripherals of
//! [`mcs_core::riscv::devices`], and the [`Target`](crate::target::Target) implementation
//! ([`target`]) the debugger session drives.
//!
//! * [`sys`] — pins, clock tree (exact rational frequencies), CPU interrupt controller, SYSTEM state.
//! * [`system`] — SYSTEM, INTERRUPT_CORE0 and RTC_CNTL register blocks.
//! * [`systimer`], [`timg`] — SYSTIMER and the timer groups.
//! * [`gpio`] — GPIO matrix + IO MUX; [`uart`], [`usb`] — UART0/1 and the USB Serial/JTAG controller.
//! * [`csr`] — custom CSRs; [`serial`], [`stimulus`] — the test-bench ends of the pins.
//!
//! # Direct boot
//!
//! There is no ROM bootloader: loading a program places its segments at their link addresses and starts at the
//! entry point with the peripherals in their reset state (CPU on XTAL_CLK / 2 = 20 MHz, interrupts disabled,
//! watchdogs off, `mie` all ones). The instruction cache / MMU are simplified: the flash image is mapped
//! linearly at IROM (0x4200_0000) and, from a 64 KiB aligned offset behind the IROM data, at DROM (0x3C00_0000).
//! Executing the boot ROM range (0x4000_0000 - 0x4005_FFFF) stops the simulation with a message.

pub mod csr;
pub mod gpio;
pub mod misc;
pub mod serial;
pub mod stimulus;
pub mod sys;
pub mod system;
pub mod systimer;
pub mod target;
pub mod timg;
pub mod uart;
pub mod usb;

use mcs_core::program::LoadedProgram;
use mcs_core::riscv::device::RiscvDeviceSpec;

use super::bus::{Bus, MemId, Mmio, BRIDGE_OWNER};
use super::machine::{Machine, RvConfig, StopReason};
use crate::avr::peripherals::serial::SerialConfig;

/// Boot ROM instruction range (empty memory; execution stops).
pub const ROM_RANGE: (u32, u32) = (0x4000_0000, 0x4006_0000);
const PERIPH_BASE: u32 = 0x6000_0000;
const PERIPH_END: u32 = 0x600d_0000;
const FLASH_ALIGN: u32 = 0x1_0000;

pub struct Esp32c3 {
    pub machine: Machine,
    pub spec: &'static RiscvDeviceSpec,
    pub(crate) flash: MemId,
    pub(crate) sram: MemId,
    pub(crate) rtc: MemId,
    /// Flash offset the DROM window starts at (see the module docs).
    pub(crate) drom_off: u32,
    /// SRAM image last sent to the UI (the next state only carries it when it changed).
    pub(crate) ram_sent: Vec<u8>,
    pub(crate) extra_sel: usize,
    pub(crate) extra_sent: Vec<u8>,
    pub(crate) extra_dirty: bool,
    /// Loaded segments that live in RAM (re-applied at every power-on, like a bootloader would).
    ram_image: Vec<(u32, Vec<u8>)>,
}

impl Esp32c3 {
    /// Builds the complete microcontroller described by `spec`; power-on reset has been applied.
    pub fn from_spec(spec: &'static RiscvDeviceSpec) -> Self {
        let (mut bus, flash, sram) = Bus::esp32c3(spec.flash_size);
        let rtc = MemId(4);
        let ps = &spec.peripheral_set;
        bus.cx.sys = sys::Sys::new(spec.gpio_count as usize);
        bus.stim = stimulus::Stimulus::new(spec.gpio_count as usize);

        // (name, base, device, reset position, listens to pins)
        type Entry = (String, u32, Box<dyn Mmio>, Option<(u8, u8)>, bool);
        let mut devs: Vec<Entry> = vec![
            ("SYSTEM".into(), ps.system_base, Box::new(system::System::new(spec)), None, false),
            ("INTERRUPT_CORE0".into(), ps.intc_base, Box::new(system::IntMatrix::new(spec)), None, false),
            ("SYSTIMER".into(), ps.systimer_base, Box::new(systimer::SysTimer::new()), None, false),
            ("RTC_CNTL".into(), ps.rtc_base, Box::new(system::Rtc::new(spec)), None, false),
            ("GPIO".into(), ps.gpio_base, Box::new(gpio::Gpio::new()), None, false),
            ("IO_MUX".into(), ps.io_mux_base, Box::new(gpio::IoMux), None, false),
            ("USB_DEVICE".into(), ps.usb_base, Box::new(usb::UsbSerial::new(spec)), Some((0, 23)), false),
        ];
        for u in &ps.uarts {
            let bit = if u.index == 0 { 2 } else { 5 };
            devs.push((u.name.clone(), u.base, Box::new(uart::Uart::new(spec, u)), Some((0, bit)), true));
        }
        for t in &ps.timgs {
            let bit = if t.index == 0 { 13 } else { 15 };
            devs.push((t.name.clone(), t.base, Box::new(timg::Timg::new(spec, t)), Some((0, bit)), false));
        }
        // Register the modelled blocks first (in this order: SYSTEM resets before the devices that read the
        // clock tree), then fill the gaps of the peripheral area with catch-all devices.
        let mut taken: Vec<(u32, u32)> = devs.iter().map(|d| (d.1, d.1 + 0x1000)).collect();
        taken.sort_unstable();
        for (name, base, dev, reset, listen) in devs {
            bus.add_named(&name, base, 0x1000, dev, reset, listen).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let mut a = PERIPH_BASE;
        for &(s, e) in taken.iter().chain(std::iter::once(&(PERIPH_END, PERIPH_END))) {
            if s > a {
                bus.add_named("(unimplemented)", a, s - a, Box::new(misc::Unimpl { base: a }), None, false).expect("peripheral gap");
            }
            a = a.max(e);
        }

        let mut machine = Machine::with_bus(RvConfig { reset_pc: spec.flash_base, ..RvConfig::default() }, bus);
        machine.halt_on_ebreak = true;
        machine.rom_range = ROM_RANGE;
        machine.csr_hook = Some(Box::<csr::EspCsrs>::default());
        let mut s = Self { machine, spec, flash, sram, rtc, drom_off: 0, ram_sent: Vec::new(), extra_sel: 0, extra_sent: Vec::new(), extra_dirty: false, ram_image: Vec::new() };
        // The UART0 pins are wired to the Serial Monitor by default.
        s.set_serial_config(SerialConfig { monitor: Some(21), inject: Some(20), baud: 115_200.0, data_bits: 8, parity: 0, stop_bits: 1 });
        s.reset_with(true);
        s
    }

    pub(crate) fn set_serial_config(&mut self, cfg: SerialConfig) {
        let b = &mut self.machine.bus;
        b.cx.owner = BRIDGE_OWNER;
        b.cx.cycles = self.machine.cpu.cycles;
        b.bridge.configure(cfg, &mut b.cx);
        self.after_io();
    }

    /// Delivers pin / clock / reset side effects queued by a host-side change.
    pub(crate) fn after_io(&mut self) {
        let now = self.machine.cpu.cycles;
        if self.machine.bus.cx.sys.attn {
            self.machine.bus.after_io(now);
        }
        self.machine.sync_irq();
    }

    /// Power-on (`power_on`) or system reset (peripherals and CPU reset, memories and time kept).
    pub fn reset_with(&mut self, power_on: bool) {
        let m = &mut self.machine;
        let (cycles, instret) = (m.cpu.cycles, m.cpu.instret);
        if power_on {
            m.bus.cx.sys.power_on();
            m.bus.mem_data_mut(self.sram).fill(0);
            m.bus.mem_data_mut(self.rtc).fill(0);
            for (addr, data) in &self.ram_image {
                m.bus.write_bytes(*addr, data);
            }
        }
        m.bus.cx.sched.clear();
        m.bus.cx.irq_raise = 0;
        m.bus.cx.irq_lower = 0;
        m.bus.cx.stop_req = false;
        m.bus.cx.sys.reset_req = false;
        m.reset();
        if !power_on {
            m.cpu.cycles = cycles;
            m.cpu.instret = instret;
        }
        let now = m.cpu.cycles;
        m.bus.reset_devices(now, power_on);
        // Strapping pins: the levels at reset.
        let sys = &mut m.bus.cx.sys;
        let mut strap = 0;
        for p in [2usize, 8, 9] {
            strap |= (sys.pins[p].level as u32) << p;
        }
        sys.gpio.strap = strap;
        m.set_irq_pending(0);
        m.cpu.csr.mie = 0xffff_fffe;
        m.sync_irq();
        let pc = m.cfg.reset_pc;
        m.set_pc(pc);
    }

    /// Flash offset of an address in the IROM / DROM windows.
    pub fn flash_offset(&self, addr: u32) -> Option<u32> {
        let s = self.spec;
        if addr >= s.flash_base && addr - s.flash_base < s.flash_size {
            return Some(addr - s.flash_base);
        }
        if addr >= s.drom_base && addr - s.drom_base < s.flash_size {
            return Some(self.drom_off + (addr - s.drom_base));
        }
        None
    }

    /// Loads a program (see the module docs): clears the flash, places every segment at its address and
    /// points the reset vector at the entry point.
    pub fn load(&mut self, p: &LoadedProgram) {
        let s = self.spec;
        let in_irom = |a: u32, n: usize| a >= s.flash_base && a - s.flash_base < s.flash_size && n > 0;
        // DROM starts behind the IROM data so IDF-style images (both windows start at +0x20) do not collide.
        let mut extent = 0u32;
        if p.segments.is_empty() {
            extent = p.flash_used;
        }
        for seg in &p.segments {
            if in_irom(seg.address, seg.data.len()) {
                extent = extent.max((seg.address - s.flash_base).saturating_add(seg.data.len() as u32));
            }
        }
        self.drom_off = if p.segments.iter().any(|g| g.address >= s.drom_base && g.address - s.drom_base < s.flash_size) { extent.div_ceil(FLASH_ALIGN) * FLASH_ALIGN } else { 0 };
        let drom_off = self.drom_off.min(s.flash_size.saturating_sub(0x1000));
        self.drom_off = drom_off;
        let m = &mut self.machine;
        m.bus.mem_data_mut(self.flash).fill(0xff);
        m.bus.set_window_offset(s.drom_base, drom_off);
        if p.segments.is_empty() {
            // A flat flash image (e.g. from a hex file): at the IROM window.
            let used = (p.flash_used as usize).min(p.flash.len());
            let base = if p.flash_base != 0 { p.flash_base } else { s.flash_base };
            m.bus.write_bytes(base, &p.flash[..used]);
        }
        for seg in &p.segments {
            m.bus.write_bytes(seg.address, &seg.data);
        }
        m.cfg.reset_pc = if p.entry != 0 { p.entry } else { s.flash_base };
        self.ram_image = p.segments.iter().filter(|g| self.flash_offset(g.address).is_none()).map(|g| (g.address, g.data.clone())).collect();
        self.ram_sent.clear();
        self.extra_dirty = true;
    }

    /// Advances time until `limit` cycles or a stop condition, dispatching scheduler events.
    pub fn run(&mut self, limit: u64) -> RunStop {
        loop {
            let m = &mut self.machine;
            let now = m.cpu.cycles;
            m.bus.service(now);
            m.sync_irq();
            if m.bus.cx.sys.reset_req {
                self.reset_with(false);
                continue;
            }
            if now >= limit {
                return RunStop::Limit;
            }
            let next = m.bus.cx.sched.next.min(limit);
            if m.is_sleeping() {
                m.idle(next.saturating_sub(now).max(1));
                continue;
            }
            match m.run(next.saturating_sub(now).max(1)) {
                StopReason::Cycles | StopReason::Wfi => {}
                StopReason::Ebreak => return RunStop::Ebreak,
                StopReason::Breakpoint => return RunStop::Breakpoint,
                StopReason::Requested => {
                    if !m.bus.cx.sys.reset_req {
                        return RunStop::Requested;
                    }
                }
                StopReason::RomCall => {
                    let (pc, ra) = (m.cpu.pc, m.cpu.x[1]);
                    let c = m.cpu.cycles;
                    m.bus.cx.sys.message(c, "error", format!("Execution entered the boot ROM at 0x{pc:08X} (return address 0x{ra:08X}). The ROM is not simulated, so ROM functions (esp_rom_printf, ets_delay_us, ...) cannot run"));
                    return RunStop::RomCall;
                }
            }
        }
    }

}

/// Why [`Esp32c3::run`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStop {
    Limit,
    Ebreak,
    Breakpoint,
    Requested,
    RomCall,
}
