//! ESP32-C3 device descriptions: memory map, pins, register addresses and interrupt sources checked against the
//! ESP32-C3 TRM / datasheet values (independent of the generator that produced the register tables).

use mcs_core::avr::device::PinKind;
use mcs_core::device::{Arch, DeviceRef};
use mcs_core::riscv::device::RiscvDeviceSpec;
use mcs_core::riscv::devices;

fn c3() -> &'static RiscvDeviceSpec {
    devices::get("esp32-c3").unwrap()
}

fn fh4() -> &'static RiscvDeviceSpec {
    devices::get("ESP32-C3FH4").unwrap()
}

#[test]
fn registry_and_identity() {
    assert_eq!(devices::list().len(), 2);
    for d in [c3(), fh4()] {
        assert_eq!((d.family.as_str(), d.package.as_str(), d.gpio_count, d.flash_size), ("ESP32-C3", "QFN32", 22, 4 << 20));
        assert_eq!((d.flash_base, d.drom_base, d.sram_base, d.sram_size, d.iram_base), (0x4200_0000, 0x3c00_0000, 0x3fc8_0000, 0x6_0000, 0x4038_0000));
        assert_eq!((d.clock.xtal_hz, d.clock.systimer_hz, d.clock.cpu_max_hz), (40e6, 16e6, 160e6));
        assert_eq!((d.vcc, d.vcc_range), (3.3, (3.0, 3.6)));
        assert_eq!(d.strapping, vec![2, 8, 9]);
        assert_eq!(d.cpu_interrupts, 31);
        assert!(d.isa.starts_with("rv32imc"));
    }
    assert!(c3().flash_external && !fh4().flash_external);
    let any = mcs_core::devices::get_any("esp32-c3").unwrap();
    assert!(matches!(any, DeviceRef::Riscv(_)));
    assert_eq!((any.arch(), any.flash_size(), any.flash_base(), any.name()), (Arch::Riscv, 4 << 20, 0x4200_0000, "ESP32-C3"));
    assert!(any.as_riscv().is_some() && any.as_avr().is_none() && any.as_arm().is_none());
    assert_eq!(mcs_core::devices::get_any("esp32-c3fh4").unwrap().name(), "ESP32-C3FH4");
    assert!(any.same_as(&mcs_core::devices::get_any("ESP32-C3").unwrap()) && !any.same_as(&mcs_core::devices::get_any("esp32-c3fh4").unwrap()));
    assert!(mcs_core::devices::list_any().iter().filter(|d| d.arch() == Arch::Riscv).count() == 2);
    let json = serde_json::to_value(any).unwrap();
    assert_eq!((json["arch"].as_str(), json["id"].as_str(), json["gpioCount"].as_u64()), (Some("riscv"), Some("esp32-c3"), Some(22)));
}

#[test]
fn memory_map_follows_the_trm() {
    let d = c3();
    let m = |n: &str| d.memory_map.iter().find(|w| w.name == n).unwrap_or_else(|| panic!("window {n}"));
    assert_eq!((m("Boot ROM (IBUS)").base, m("Boot ROM (IBUS)").size), (0x4000_0000, 384 << 10));
    assert_eq!((m("Boot ROM data (DBUS)").base, m("Boot ROM data (DBUS)").size), (0x3ff0_0000, 128 << 10));
    assert_eq!((m("SRAM0 (IRAM)").base, m("SRAM0 (IRAM)").size), (0x4037_c000, 16 << 10));
    assert_eq!((m("SRAM1 (IRAM)").base, m("SRAM1 (IRAM)").size, m("SRAM1 (IRAM)").perm.as_str()), (0x4038_0000, 384 << 10, "rwx"));
    assert_eq!((m("SRAM1 (DRAM)").base, m("SRAM1 (DRAM)").perm.as_str()), (0x3fc8_0000, "rw-"));
    assert_eq!((m("RTC FAST memory").base, m("RTC FAST memory").size), (0x5000_0000, 8 << 10));
    assert_eq!((m("Flash (IROM)").base, m("Flash (IROM)").perm.as_str()), (0x4200_0000, "r-x"));
    assert_eq!((m("Flash (DROM)").base, m("Flash (DROM)").perm.as_str()), (0x3c00_0000, "r--"));
    assert_eq!(d.extra_ram.len(), 2);
    assert_eq!((d.extra_ram[0].base, d.extra_ram[0].size, d.extra_ram[1].base, d.extra_ram[1].size), (0x4037_c000, 0x4000, 0x5000_0000, 0x2000));
}

#[test]
fn qfn32_pins() {
    let d = c3();
    assert_eq!(d.pins.len(), 33, "32 pins plus the exposed pad");
    let pin = |n: u8| d.pins.iter().find(|p| p.number == n).unwrap();
    assert_eq!((pin(28).name.as_str(), pin(28).gpio, pin(27).name.as_str(), pin(27).gpio), ("U0TXD", Some(21), "U0RXD", Some(20)));
    assert_eq!((pin(6).gpio, pin(14).gpio, pin(15).gpio), (Some(2), Some(8), Some(9)));
    assert!(pin(6).functions.iter().any(|f| f == "strapping") && pin(15).functions.iter().any(|f| f.contains("strapping")));
    assert_eq!((pin(7).name.as_str(), pin(17).name.as_str(), pin(18).name.as_str()), ("CHIP_EN", "VDD3P3_CPU", "VDD_SPI"));
    assert!(matches!(pin(2).kind, PinKind::Vcc) && matches!(pin(33).kind, PinKind::Gnd));
    // Every GPIO 0 - 21 except GPIO11 (VDD_SPI) is on exactly one pin.
    let mut gpios: Vec<u8> = d.pins.iter().filter_map(|p| p.gpio).collect();
    gpios.sort();
    assert_eq!(gpios, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21]);
    // The in-package flash of the FH4 takes the SPI flash pins (GPIO12 - GPIO17).
    let f: Vec<u8> = fh4().pins.iter().filter_map(|p| p.gpio).collect();
    assert!(f.iter().all(|&g| !(12..=17).contains(&g)) && f.len() == 15, "{f:?}");
    assert!(fh4().pins.iter().any(|p| p.number == 18 && p.name.contains("in-package flash")));
}

#[test]
fn register_addresses_and_reset_values() {
    let d = c3();
    for (name, addr, reset) in [
        ("UART0_FIFO", 0x6000_0000, 0),
        ("UART0_CLKDIV", 0x6000_0014, 0x2b6),
        ("UART0_CLK_CONF", 0x6000_0078, 0x0370_1000),
        ("UART1_CLKDIV", 0x6001_0014, 0x2b6),
        ("GPIO_OUT_W1TS", 0x6000_4008, 0),
        ("GPIO_IN", 0x6000_403c, 0),
        ("GPIO_FUNC6_OUT_SEL_CFG", 0x6000_4554 + 4 * 6, 0x80),
        ("GPIO_PIN21", 0x6000_4074 + 4 * 21, 0),
        ("IO_MUX_GPIO0", 0x6000_9004, 0xb00),
        ("IO_MUX_GPIO21", 0x6000_9058, 0xb00),
        ("SYSTIMER_UNIT0_VALUE_LO", 0x6002_3044, 0),
        ("TIMG0_T0CONFIG", 0x6001_f000, 0x6000_2000),
        ("TIMG1_T0CONFIG", 0x6002_0000, 0x6000_2000),
        ("SYSTEM_SYSCLK_CONF", 0x600c_0058, 1),
        ("SYSTEM_CPU_PER_CONF", 0x600c_0008, 0xc),
        ("INTERRUPT_CORE0_CPU_INT_ENABLE", 0x600c_2104, 0),
        ("INTERRUPT_CORE0_GPIO_INTERRUPT_PRO_MAP", 0x600c_2040, 0),
        ("USB_DEVICE_EP1", 0x6004_3000, 0),
        ("RTC_CNTL_WDTCONFIG0", 0x6000_8090, 0x0001_3214),
    ] {
        let r = d.register(name).unwrap_or_else(|| panic!("register {name}"));
        assert_eq!((r.addr, r.reset), (addr, reset), "{name}");
    }
    assert!(d.registers.windows(2).all(|w| w[0].addr <= w[1].addr), "sorted by address");
    // Bit fields come with masks.
    let conf = d.register("TIMG0_T0CONFIG").unwrap();
    assert_eq!(conf.bits.iter().find(|b| b.name == "EN").unwrap().mask, 1 << 31);
    assert_eq!(conf.bits.iter().find(|b| b.name == "DIVIDER").unwrap().mask, 0xffff << 13);
    // Group names match the declared groups.
    for r in &d.registers {
        assert!(d.groups.iter().any(|g| g.name == r.group), "{} has group {}", r.name, r.group);
    }
}

#[test]
fn interrupt_sources_match_the_matrix() {
    let d = c3();
    let src = |n: &str| d.interrupts.iter().find(|i| i.name == n).unwrap_or_else(|| panic!("source {n}")).source;
    assert_eq!((src("GPIO"), src("UART0"), src("UART1"), src("USB_DEVICE"), src("SYSTIMER_TARGET0"), src("TG0_T0_LEVEL"), src("TG1_T0_LEVEL"), src("FROM_CPU_INTR0")), (16, 21, 22, 26, 37, 32, 34, 50));
    // Source n is mapped by the INTERRUPT_CORE0 register at offset 4 * n.
    let base = d.reg("INTERRUPT_CORE0_MAC_INTR_MAP");
    assert_eq!(base, 0x600c_2000);
    assert_eq!(d.reg("INTERRUPT_CORE0_GPIO_INTERRUPT_PRO_MAP"), base + 4 * 16);
    assert_eq!(d.reg("INTERRUPT_CORE0_UART_INTR_MAP"), base + 4 * 21);
    assert_eq!(d.reg("INTERRUPT_CORE0_SYSTIMER_TARGET0_INT_MAP"), base + 4 * 37);
    assert_eq!(d.reg("INTERRUPT_CORE0_CPU_INTR_FROM_CPU_0_MAP"), base + 4 * 50);
    // Peripheral wiring recipe.
    let ps = &d.peripheral_set;
    assert_eq!((ps.uarts.len(), ps.uarts[0].source, ps.uarts[1].source, ps.uarts[1].base), (2, 21, 22, 0x6001_0000));
    assert_eq!((ps.timgs[0].t0_source, ps.timgs[0].wdt_source, ps.timgs[1].t0_source, ps.timgs[1].base), (32, 33, 34, 0x6002_0000));
}
