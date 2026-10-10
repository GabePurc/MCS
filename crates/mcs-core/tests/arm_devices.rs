//! STM32G4 device descriptions: addresses, vectors, packages and alternate functions checked
//! against RM0440 / DS12589 / DS12288 values (independent of the generator that produced the tables).

use mcs_core::arm::device::ArmDeviceSpec;
use mcs_core::arm::devices;
use mcs_core::avr::device::PinKind;
use mcs_core::device::{Arch, DeviceRef};

fn g474() -> &'static ArmDeviceSpec {
    devices::get("stm32g474re").unwrap()
}

fn g431() -> &'static ArmDeviceSpec {
    devices::get("STM32G431KB").unwrap()
}

#[test]
fn memories_and_identity() {
    let (a, b) = (g474(), g431());
    assert_eq!((a.flash_size, a.sram_size, a.ccm_sram.unwrap().size, a.ram_total()), (512 * 1024, 96 * 1024, 32 * 1024, 128 * 1024));
    assert_eq!((b.flash_size, b.sram_size, b.ccm_sram.unwrap().size, b.ram_total()), (128 * 1024, 22 * 1024, 10 * 1024, 32 * 1024));
    for d in [a, b] {
        assert_eq!((d.flash_base, d.sram_base, d.ccm_sram.unwrap().base), (0x0800_0000, 0x2000_0000, 0x1000_0000));
        assert_eq!(d.ccm_sram.unwrap().alias_base, d.sram_base + d.sram_size, "CCM follows SRAM1/SRAM2");
        assert_eq!((d.core_name.as_str(), d.family.as_str(), d.nvic_prio_bits, d.nirq, d.cpuid), ("Cortex-M4F", "STM32G4", 4, 102, 0x410F_C241));
        assert_eq!((d.clock.hsi_hz, d.clock.lsi_hz), (16e6, 32e3));
        assert_eq!(d.speed_grades, vec![(170e6, 1.71)]);
        assert_eq!(d.gpio_count, 112);
    }
    assert_eq!((a.package.as_str(), b.package.as_str()), ("LQFP64", "LQFP32"));
    assert!(matches!(mcs_core::devices::get_any("stm32g474re"), Some(DeviceRef::Arm(_))));
    assert_eq!(mcs_core::devices::get_any("stm32g474re").unwrap().arch(), Arch::Arm);
}

#[test]
fn register_addresses() {
    let d = g474();
    for (name, addr) in [
        ("RCC_CR", 0x4002_1000),
        ("RCC_PLLCFGR", 0x4002_100C),
        ("RCC_AHB2ENR", 0x4002_104C),
        ("RCC_APB1ENR1", 0x4002_1058),
        ("RCC_APB2ENR", 0x4002_1060),
        ("FLASH_ACR", 0x4002_2000),
        ("PWR_CR5", 0x4000_7080),
        ("GPIOA_MODER", 0x4800_0000),
        ("GPIOB_ODR", 0x4800_0414),
        ("GPIOG_BSRR", 0x4800_1818),
        ("GPIOC_AFRH", 0x4800_0824),
        ("SYSCFG_EXTICR1", 0x4001_0008),
        ("EXTI_PR1", 0x4001_0414),
        ("USART1_BRR", 0x4001_380C),
        ("USART2_TDR", 0x4000_4428),
        ("UART5_ISR", 0x4000_501C),
        ("LPUART1_CR1", 0x4000_8000),
        ("TIM2_CNT", 0x4000_0024),
        ("TIM3_CCR1", 0x4000_0434),
        ("TIM4_ARR", 0x4000_082C),
        ("TIM7_PSC", 0x4000_1428),
        ("SysTick_CSR", 0xE000_E010),
        ("SCB_VTOR", 0xE000_ED08),
        ("NVIC_ISER0", 0xE000_E100),
        ("NVIC_IPR25", 0xE000_E464),
    ] {
        assert_eq!(d.reg(name), addr, "{name}");
    }
    assert!(g431().register("UART5_ISR").is_none() && g431().register("UART4_ISR").is_some());
    assert!(d.registers.windows(2).all(|w| w[0].addr <= w[1].addr), "sorted by address");
    // Reset values and bit fields.
    let reg = |n: &str| d.register(n).unwrap();
    assert_eq!(reg("GPIOA_MODER").reset, 0xABFF_FFFF);
    assert_eq!(reg("GPIOB_MODER").reset, 0xFFFF_FEBF);
    assert_eq!(reg("GPIOC_MODER").reset, 0xFFFF_FFFF);
    assert_eq!(reg("RCC_CR").reset, 0x500);
    assert_eq!(reg("TIM2_ARR").reset, 0xFFFF_FFFF);
    assert_eq!(reg("TIM3_ARR").reset, 0xFFFF);
    let hsion = reg("RCC_CR").bits.iter().find(|b| b.name == "HSION").unwrap();
    assert_eq!(hsion.mask, 1 << 8);
    assert!(reg("TIM2_DIER").bits.iter().any(|b| b.name == "UIE" && b.mask == 1));
    assert!(reg("USART1_CR1").bits.iter().any(|b| b.name == "TE" && b.mask == 1 << 3));
    assert!(reg("GPIOA_MODER").bits.iter().any(|b| b.name == "MODE5" && b.mask == 3 << 10));
    assert_eq!(reg("GPIOA_IDR").access, mcs_core::avr::device::RegisterAccess::R);
    assert!(g431().register("TIM5_CNT").is_none());
}

#[test]
fn vectors_use_exception_numbers() {
    let d = g474();
    let v = |n: &str| d.vectors.iter().find(|v| v.name == n).unwrap_or_else(|| panic!("vector {n}")).index;
    assert_eq!((v("Reset"), v("NMI"), v("HardFault"), v("SVCall"), v("PendSV"), v("SysTick")), (1, 2, 3, 11, 14, 15));
    assert_eq!((v("WWDG"), v("EXTI0"), v("EXTI9_5"), v("TIM2"), v("TIM3"), v("USART1"), v("EXTI15_10"), v("UART5"), v("LPUART1")), (16, 22, 39, 44, 45, 53, 56, 69, 107));
    assert!(g431().vectors.iter().all(|x| x.name != "UART5" && x.name != "TIM5"));
    let ps = &d.peripheral_set;
    assert_eq!(ps.exti_irqs, vec![6, 7, 8, 9, 10, 23, 23, 23, 23, 23, 40, 40, 40, 40, 40, 40]);
    assert_eq!(ps.uarts.iter().map(|u| (u.name.as_str(), u.irq)).collect::<Vec<_>>(), [("USART1", 37), ("USART2", 38), ("USART3", 39), ("UART4", 52), ("UART5", 53), ("LPUART1", 91)]);
    assert_eq!(ps.timers.iter().map(|t| (t.name.as_str(), t.irq, t.width)).collect::<Vec<_>>(), [("TIM2", 28, 32), ("TIM3", 29, 16), ("TIM4", 30, 16), ("TIM6", 54, 16), ("TIM7", 55, 16)]);
}

#[test]
fn package_pins_and_alternate_functions() {
    for (d, count, pa5, pa9) in [(g474(), 64, 19, 43), (g431(), 32, 10, 19)] {
        assert_eq!(d.pins.len(), count, "{} pins", d.package);
        let mut numbers: Vec<u8> = d.pins.iter().map(|p| p.number).collect();
        numbers.sort_unstable();
        assert!(numbers.iter().copied().eq(1..=count as u8), "pin numbers 1..={count}");
        let pin = |n: &str| d.pins.iter().find(|p| p.name == n).unwrap_or_else(|| panic!("pin {n}"));
        assert_eq!((pin("PA5").number, pin("PA5").gpio, pin("PA5").kind), (pa5, Some(5), PinKind::Io));
        assert_eq!(pin("PA9").number, pa9);
        assert!(pin("PA9").functions.iter().any(|f| f == "USART1_TX"));
        assert!(d.pins.iter().any(|p| p.kind == PinKind::Vcc) && d.pins.iter().any(|p| p.kind == PinKind::Gnd));
        let af = &d.peripheral_set.alt_functions;
        // PA9 = USART1_TX on AF7, PA6 = TIM3_CH1 on AF2, PA0 = TIM2_CH1 on AF1.
        assert!(af.contains(&(9, 7, "USART1_TX")) && af.contains(&(6, 2, "TIM3_CH1")) && af.contains(&(0, 1, "TIM2_CH1")));
        // Every routed pin exists in the package.
        let gpios: Vec<u8> = d.pins.iter().filter_map(|p| p.gpio).collect();
        assert!(af.iter().all(|a| gpios.contains(&a.0)));
    }
    assert_eq!(g474().gpio_names()[5], "PA5");
    assert_eq!(g474().gpio_names()[111], "PG15");
}

#[test]
fn spec_serializes_with_the_arch_tag() {
    let j = serde_json::to_value(mcs_core::devices::get_any("stm32g474re").unwrap()).unwrap();
    assert_eq!(j["arch"], "arm");
    assert_eq!(j["flashBase"], 0x0800_0000u32);
    assert_eq!(j["ccmSram"]["aliasBase"], 0x2001_8000u32);
    assert!(j["registers"][0]["group"].is_string());
    assert!(j["peripheralSet"]["gpio"].as_array().unwrap().len() == 7);
    assert!(j["vectors"].as_array().unwrap().iter().any(|v| v["name"] == "SysTick" && v["index"] == 15));
}
