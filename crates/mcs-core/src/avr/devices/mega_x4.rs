//! ATmega164PA / ATmega324PA / ATmega644PA / ATmega1284P (classic AVR core, PDIP-40): four 8-bit
//! ports A-D, two USARTs, 8-bit Timer/Counter0 and 2, 16-bit Timer/Counter1 (and 3 on the
//! ATmega1284P), SPI, TWI, 8-channel ADC with differential/gain channels, analog comparator,
//! INT0-2, pin change interrupts PCINT0-31 (ports A-D), PRR0 (and PRR1 on the ATmega1284P).
//!
//! Sources: Atmel-8272G "ATmega164A/PA/324A/PA/644A/PA/1284/P" (01/2015) register summary,
//! pinout, interrupt vector table, fuse and boot loader tables, speed grades; avr-libc
//! iomxx4.h / iom164pa.h / iom324pa.h / iom644pa.h / iom1284p.h for register addresses, bit names,
//! signatures, vector numbers and memory sizes.
//!
//! Datasheet / avr-libc conflicts (the datasheet text wins):
//! - Fuse factory defaults are low 0x62 (CKDIV8 programmed, SUT = 10, CKSEL = 0010), high 0x99
//!   (JTAGEN and SPIEN programmed, BOOTSZ = 00) and extended 0xFF. avr-libc's `LFUSE_DEFAULT`
//!   programs both SUT bits (0x42) and the ATmega1284P `HFUSE_DEFAULT` leaves BOOTSZ1 unprogrammed
//!   (0x9D); iom324pa.h / iom644pa.h agree with 0x99.
//!
//! Deliberate simplifications:
//! - JTAG and on-chip debug: the JTAGEN fuse is programmed at the factory, which takes PC2-PC5 on
//!   real silicon; here those pins stay ordinary GPIO. JTD, JTRF, OCDEN and OCDR exist as
//!   registers/fuses but have no function.
//! - Timer/Counter2 asynchronous operation (ASSR.AS2, TOSC1/TOSC2 on PC6/PC7) is not modelled; the
//!   timer runs from the system clock and keeps counting in power-save like the ATmega328P model.
//! - The ATmega1284P pinout has no T3 pin, so Timer/Counter3 cannot count external pulses.
//! - The dedicated RESET pin has no GPIO slot (all 32 GPIOs are the ports), so an external reset
//!   cannot be driven from the pin panel.
//! - USART synchronous (UMSEL) and master SPI modes are not simulated; SPM is not simulated.

use super::mega_big::*;
use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash: u32,
    sram: u16,
    eeprom: u16,
    signature: [u8; 3],
    /// Boot section sizes in words per BOOTSZ value.
    boot: [u32; 4],
    /// ATmega1284P: 128 KB flash (ELPM, RAMPZ), Timer/Counter3 and PRR1.
    big: bool,
}

const VARIANTS: &[Variant] = &[
    Variant { id: "atmega164pa", name: "ATmega164PA", flash: 16384, sram: 1024, eeprom: 512, signature: [0x1e, 0x94, 0x0a], boot: [1024, 512, 256, 128], big: false },
    Variant { id: "atmega324pa", name: "ATmega324PA", flash: 32768, sram: 2048, eeprom: 1024, signature: [0x1e, 0x95, 0x11], boot: [2048, 1024, 512, 256], big: false },
    Variant { id: "atmega644pa", name: "ATmega644PA", flash: 65536, sram: 4096, eeprom: 2048, signature: [0x1e, 0x96, 0x0a], boot: [4096, 2048, 1024, 512], big: false },
    Variant { id: "atmega1284p", name: "ATmega1284P", flash: 131072, sram: 16384, eeprom: 4096, signature: [0x1e, 0x97, 0x05], boot: [4096, 2048, 1024, 512], big: true },
];

fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let mut r = ports(&[('A', 0x20, 0xff), ('B', 0x23, 0xff), ('C', 0x26, 0xff), ('D', 0x29, 0xff)]);
    r.extend(timer8(0, 0x44, 0x35, 0x6e));
    r.extend(timer16(1, 0x80, 0x36, 0x6f, false));
    r.extend(timer8(2, 0xb0, 0x37, 0x70));
    r.push(assr());
    if v.big {
        r.extend(timer16(3, 0x90, 0x38, 0x71, false));
    }
    r.push(reg("PCIFR", 0x3b, "EXINT", "Pin Change Interrupt Flag Register", (0..4).rev().map(|i| b(&format!("PCIF{i}"), 1 << i, &format!("Pin Change Interrupt Flag {i} (PCINT{}..{})", i * 8 + 7, i * 8))).collect()));
    r.push(reg("EIFR", 0x3c, "EXINT", "External Interrupt Flag Register", (0..3).rev().map(|i| b(&format!("INTF{i}"), 1 << i, &format!("External Interrupt Flag {i}"))).collect()));
    r.push(reg("EIMSK", 0x3d, "EXINT", "External Interrupt Mask Register", (0..3).rev().map(|i| b(&format!("INT{i}"), 1 << i, &format!("External Interrupt Request {i} Enable"))).collect()));
    r.extend(eeprom());
    r.push(reg("GTCCR", 0x43, "TC0", "General Timer/Counter Control Register", vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PSRASY", 0x02, "Prescaler Reset Timer/Counter2"), b("PSRSYNC", 0x01, "Prescaler Reset synchronous Timer/Counters")]));
    r.extend(spi());
    r.extend(analog_comparator());
    let prr0 = vec![
        b("PRTWI", 0x80, "Power Reduction TWI"), b("PRTIM2", 0x40, "Power Reduction Timer/Counter2"), b("PRTIM0", 0x20, "Power Reduction Timer/Counter0"),
        b("PRUSART1", 0x10, "Power Reduction USART1"), b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRSPI", 0x04, "Power Reduction SPI"),
        b("PRUSART0", 0x02, "Power Reduction USART0"), b("PRADC", 0x01, "Power Reduction ADC"),
    ];
    r.extend(cpu(&Cpu {
        sram_end: (0x100 + v.sram as u32 - 1) as u16,
        bod_sleep: true,
        rampz_mask: if v.big { 0x01 } else { 0 },
        eind: false,
        prr0,
        prr1: if v.big { vec![b("PRTIM3", 0x01, "Power Reduction Timer/Counter3")] } else { vec![] },
    }));
    r.push(reg("PCICR", 0x68, "EXINT", "Pin Change Interrupt Control Register", (0..4).rev().map(|i| b(&format!("PCIE{i}"), 1 << i, &format!("Pin Change Interrupt Enable {i}"))).collect()));
    r.push(reg("EICRA", 0x69, "EXINT", "External Interrupt Control Register A", (0..3).rev().map(|i| b(&format!("ISC{i}"), 3 << (2 * i), &format!("Interrupt Sense Control {i} (00 low, 01 any, 10 falling, 11 rising)"))).collect()));
    for g in 0..4u16 {
        let name = format!("PCMSK{g}");
        let port = ['A', 'B', 'C', 'D'][g as usize];
        let addr = if g < 3 { 0x6b + g } else { 0x73 };
        r.push(reg(&name, addr, "EXINT", &format!("Pin Change Mask Register {g} (P{port}7..P{port}0)"), (g * 8..g * 8 + 8).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i % 8), "")).collect()));
    }
    r.extend(adc(false));
    r.push(reg("DIDR0", 0x7e, "ADC", "Digital Input Disable Register 0", (0..8).rev().map(|i| field(&format!("ADC{i}D"), 1 << i, "")).collect()));
    r.extend(twi());
    r.extend(usart(0, 0xc0));
    r.extend(usart(1, 0xc8));
    r.sort_by_key(|x| x.addr);
    r
}

fn vector_names(v: &Variant) -> Vec<String> {
    let mut n: Vec<String> = ["RESET", "INT0", "INT1", "INT2", "PCINT0", "PCINT1", "PCINT2", "PCINT3", "WDT", "TIMER2_COMPA", "TIMER2_COMPB", "TIMER2_OVF"].map(String::from).into();
    n.extend(timer_vectors(1, false));
    n.extend(["TIMER0_COMPA", "TIMER0_COMPB", "TIMER0_OVF", "SPI_STC"].map(String::from));
    n.extend(usart_vectors(0));
    n.extend(["ANALOG_COMP", "ADC", "EE_READY", "TWI", "SPM_READY"].map(String::from));
    n.extend(usart_vectors(1));
    if v.big {
        n.extend(timer_vectors(3, false));
    }
    n
}

/// PDIP-40 (Atmel-8272G figure 1-1). GPIO numbering: PA0-7 = 0-7, PB0-7 = 8-15, PC0-7 = 16-23,
/// PD0-7 = 24-31.
fn pins(v: &Variant) -> Vec<PinSpec> {
    let t3 = |f: &'static str| if v.big { Some(f) } else { None };
    let with = |base: &[&'static str], extra: Option<&'static str>| -> Vec<&'static str> { base.iter().copied().chain(extra).collect() };
    vec![
        io(1, "PB0", 8, &["XCK0", "T0", "PCINT8"]),
        io(2, "PB1", 9, &["CLKO", "T1", "PCINT9"]),
        io(3, "PB2", 10, &["INT2", "AIN0", "PCINT10"]),
        io(4, "PB3", 11, &["OC0A", "AIN1", "PCINT11"]),
        io(5, "PB4", 12, &["SS", "OC0B", "PCINT12"]),
        io(6, "PB5", 13, &with(&["MOSI", "PCINT13"], t3("ICP3"))),
        io(7, "PB6", 14, &with(&["MISO", "PCINT14"], t3("OC3A"))),
        io(8, "PB7", 15, &with(&["SCK", "PCINT15"], t3("OC3B"))),
        dedicated(9, "RESET"),
        power(10, "VCC", PinKind::Vcc),
        power(11, "GND", PinKind::Gnd),
        dedicated(12, "XTAL2"),
        dedicated(13, "XTAL1"),
        io(14, "PD0", 24, &["RXD0", "PCINT24"]),
        io(15, "PD1", 25, &["TXD0", "PCINT25"]),
        io(16, "PD2", 26, &["RXD1", "INT0", "PCINT26"]),
        io(17, "PD3", 27, &["TXD1", "INT1", "PCINT27"]),
        io(18, "PD4", 28, &["XCK1", "OC1B", "PCINT28"]),
        io(19, "PD5", 29, &["OC1A", "PCINT29"]),
        io(20, "PD6", 30, &["ICP1", "OC2B", "PCINT30"]),
        io(21, "PD7", 31, &["OC2A", "PCINT31"]),
        io(22, "PC0", 16, &["SCL", "PCINT16"]),
        io(23, "PC1", 17, &["SDA", "PCINT17"]),
        io(24, "PC2", 18, &["TCK", "PCINT18"]),
        io(25, "PC3", 19, &["TMS", "PCINT19"]),
        io(26, "PC4", 20, &["TDO", "PCINT20"]),
        io(27, "PC5", 21, &["TDI", "PCINT21"]),
        io(28, "PC6", 22, &["TOSC1", "PCINT22"]),
        io(29, "PC7", 23, &["TOSC2", "PCINT23"]),
        power(30, "AVCC", PinKind::Ref),
        power(31, "GND", PinKind::Gnd),
        power(32, "AREF", PinKind::Ref),
        io(33, "PA7", 7, &["ADC7", "PCINT7"]),
        io(34, "PA6", 6, &["ADC6", "PCINT6"]),
        io(35, "PA5", 5, &["ADC5", "PCINT5"]),
        io(36, "PA4", 4, &["ADC4", "PCINT4"]),
        io(37, "PA3", 3, &["ADC3", "PCINT3"]),
        io(38, "PA2", 2, &["ADC2", "PCINT2"]),
        io(39, "PA1", 1, &["ADC1", "PCINT1"]),
        io(40, "PA0", 0, &["ADC0", "PCINT0"]),
    ]
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let mut groups = vec![
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTA", "I/O Port A"),
        ("PORTB", "I/O Port B"),
        ("PORTC", "I/O Port C"),
        ("PORTD", "I/O Port D"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("TC1", "16-bit Timer/Counter1 with PWM"),
        ("TC2", "8-bit Timer/Counter2 with PWM (asynchronous mode not simulated)"),
    ];
    if v.big {
        groups.push(("TC3", "16-bit Timer/Counter3 with PWM"));
    }
    groups.extend([
        ("USART0", "USART0 (serial port)"),
        ("USART1", "USART1 (serial port)"),
        ("SPI", "Serial Peripheral Interface"),
        ("TWI", "2-wire Serial Interface (I2C)"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ]);
    let mut features = feature::MOVW | feature::MUL | feature::LPMX | feature::SPM | feature::BREAK | feature::JMP;
    if v.big {
        features |= feature::ELPM | feature::ELPMX | feature::SPMX;
    }
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "megaAVR (ATmega164/324/644/1284)".into(),
        core_name: if v.big { "AVRe+ (AVR51)".into() } else { "AVRe+ (AVR5)".into() },
        features,
        flash_size: v.flash,
        sram_start: 0x100,
        sram_size: v.sram,
        eeprom_size: v.eeprom,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: v.signature,
        calibration: 0x9a,
        fuses: fuses(0x99),
        sleep: sleep(),
        boot: Some(BootSpec { sizes_words: v.boot }),
        vectors: vectors(&vector_names(v)),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-40".into(),
        pins: pins(v),
        gpio_count: 32,
        has_adc: true,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // "Speed grades": 4 MHz @ 1.8 V, 10 MHz @ 2.7 V, 20 MHz @ 4.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "Atmel-8272G ATmega164A/PA/324A/PA/644A/PA/1284/P (01/2015)".into(),
        die: None,
        peripheral_set: PeripheralSet::MegaX4,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
