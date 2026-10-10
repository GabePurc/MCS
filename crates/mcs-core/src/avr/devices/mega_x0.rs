//! ATmega640 / ATmega1280 / ATmega2560 (classic AVR core, TQFP-100): eleven ports A-L (no I; 86
//! I/O lines), four USARTs, 8-bit Timer/Counter0 and 2, 16-bit Timer/Counter1/3/4/5 with three
//! output compare units (A/B/C), SPI, TWI, 16-channel ADC (MUX5 in ADCSRB) with differential/gain
//! channels, analog comparator, INT0-7, pin change interrupts PCINT0-23, PRR0/PRR1, RAMPZ and (on
//! the ATmega2560) EIND with EIJMP/EICALL and a 22-bit program counter.
//!
//! Sources: Atmel-2549Q "ATmega640/V-1280/V-1281/V-2560/V-2561/V" (02/2014) register summary,
//! pinout (TQFP-100), interrupt vector table, fuse and boot loader tables, speed grades; avr-libc
//! iomxx0_1.h / iom640.h / iom1280.h / iom2560.h for register addresses, bit names, signatures,
//! vector numbers and memory sizes.
//!
//! Datasheet / avr-libc conflicts (the datasheet text wins):
//! - iomxx0_1.h declares EIND (and only RAMPZ0) for all three parts. EIJMP/EICALL exist only on
//!   the ATmega2560 (more than 128K words of flash), so EIND is defined only there; RAMPZ has two
//!   bits on the ATmega2560 (256 KB need an 18-bit ELPM address) and one on the others.
//!
//! Deliberate simplifications:
//! - External memory interface (XMEM): XMCRA/XMCRB exist as plain registers; ports A, C and G
//!   always behave as ordinary I/O and no external SRAM is attached.
//! - JTAG and on-chip debug: the JTAGEN fuse is programmed at the factory, which takes PF4-PF7 on
//!   real silicon; here those pins stay ordinary GPIO. JTD, JTRF, OCDEN and OCDR exist but have no
//!   function.
//! - Timer/Counter2 asynchronous operation (ASSR.AS2, TOSC1/TOSC2 on PG4/PG3) is not modelled.
//! - The dedicated RESET pin has no GPIO slot, so an external reset cannot be driven from the pin
//!   panel.
//! - USART synchronous (UMSEL) and master SPI modes are not simulated; SPM is not simulated.
//! - GPIO numbering is dense (PG has 6 pins): A 0-7, B 8-15, C 16-23, D 24-31, E 32-39, F 40-47,
//!   G 48-53, H 54-61, J 62-69, K 70-77, L 78-85.

use super::mega_big::*;
use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash: u32,
    signature: [u8; 3],
}

const VARIANTS: &[Variant] = &[
    Variant { id: "atmega640", name: "ATmega640", flash: 65536, signature: [0x1e, 0x96, 0x08] },
    Variant { id: "atmega1280", name: "ATmega1280", flash: 131072, signature: [0x1e, 0x97, 0x03] },
    Variant { id: "atmega2560", name: "ATmega2560", flash: 262144, signature: [0x1e, 0x98, 0x01] },
];

const SRAM: u16 = 8192;
const SRAM_START: u16 = 0x200;

/// (port letter, PINx address, implemented pins mask, first GPIO index).
pub const PORTS: [(char, u16, u8, u8); 11] = [
    ('A', 0x20, 0xff, 0),
    ('B', 0x23, 0xff, 8),
    ('C', 0x26, 0xff, 16),
    ('D', 0x29, 0xff, 24),
    ('E', 0x2c, 0xff, 32),
    ('F', 0x2f, 0xff, 40),
    ('G', 0x32, 0x3f, 48),
    ('H', 0x100, 0xff, 54),
    ('J', 0x103, 0xff, 62),
    ('K', 0x106, 0xff, 70),
    ('L', 0x109, 0xff, 78),
];

fn gpio(port: char, bit: u8) -> u8 {
    PORTS.iter().find(|p| p.0 == port).map(|p| p.3).expect("port") + bit
}

fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let mut r = ports(&PORTS.map(|(l, a, m, _)| (l, a, m)));
    r.extend(timer8(0, 0x44, 0x35, 0x6e));
    r.extend(timer16(1, 0x80, 0x36, 0x6f, true));
    r.extend(timer8(2, 0xb0, 0x37, 0x70));
    r.push(assr());
    r.extend(timer16(3, 0x90, 0x38, 0x71, true));
    r.extend(timer16(4, 0xa0, 0x39, 0x72, true));
    r.extend(timer16(5, 0x120, 0x3a, 0x73, true));
    r.push(reg("PCIFR", 0x3b, "EXINT", "Pin Change Interrupt Flag Register", vec![
        b("PCIF2", 0x04, "Pin Change Interrupt Flag 2 (PCINT23..16)"), b("PCIF1", 0x02, "Pin Change Interrupt Flag 1 (PCINT15..8)"), b("PCIF0", 0x01, "Pin Change Interrupt Flag 0 (PCINT7..0)"),
    ]));
    r.push(reg("EIFR", 0x3c, "EXINT", "External Interrupt Flag Register", (0..8).rev().map(|i| b(&format!("INTF{i}"), 1 << i, &format!("External Interrupt Flag {i}"))).collect()));
    r.push(reg("EIMSK", 0x3d, "EXINT", "External Interrupt Mask Register", (0..8).rev().map(|i| b(&format!("INT{i}"), 1 << i, &format!("External Interrupt Request {i} Enable"))).collect()));
    r.extend(eeprom());
    r.push(reg("GTCCR", 0x43, "TC0", "General Timer/Counter Control Register", vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PSRASY", 0x02, "Prescaler Reset Timer/Counter2"), b("PSRSYNC", 0x01, "Prescaler Reset synchronous Timer/Counters")]));
    r.extend(spi());
    r.extend(analog_comparator());
    let prr0 = vec![
        b("PRTWI", 0x80, "Power Reduction TWI"), b("PRTIM2", 0x40, "Power Reduction Timer/Counter2"), b("PRTIM0", 0x20, "Power Reduction Timer/Counter0"),
        b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRSPI", 0x04, "Power Reduction SPI"), b("PRUSART0", 0x02, "Power Reduction USART0"), b("PRADC", 0x01, "Power Reduction ADC"),
    ];
    let prr1 = vec![
        b("PRTIM5", 0x20, "Power Reduction Timer/Counter5"), b("PRTIM4", 0x10, "Power Reduction Timer/Counter4"), b("PRTIM3", 0x08, "Power Reduction Timer/Counter3"),
        b("PRUSART3", 0x04, "Power Reduction USART3"), b("PRUSART2", 0x02, "Power Reduction USART2"), b("PRUSART1", 0x01, "Power Reduction USART1"),
    ];
    let big = v.flash > 131072;
    r.extend(cpu(&Cpu {
        sram_end: SRAM_START + SRAM - 1,
        bod_sleep: false,
        rampz_mask: if big { 0x03 } else { 0x01 },
        eind: big,
        prr0,
        prr1,
    }));
    r.push(reg("PCICR", 0x68, "EXINT", "Pin Change Interrupt Control Register", vec![b("PCIE2", 0x04, "Pin Change Interrupt Enable 2"), b("PCIE1", 0x02, "Pin Change Interrupt Enable 1"), b("PCIE0", 0x01, "Pin Change Interrupt Enable 0")]));
    let isc = |lo: u8| -> Vec<BitFieldSpec> { (lo..lo + 4).rev().map(|i| b(&format!("ISC{i}"), 3 << (2 * (i - lo)), &format!("Interrupt Sense Control {i} (00 low, 01 any, 10 falling, 11 rising)"))).collect() };
    r.push(reg("EICRA", 0x69, "EXINT", "External Interrupt Control Register A", isc(0)));
    r.push(reg("EICRB", 0x6a, "EXINT", "External Interrupt Control Register B", isc(4)));
    r.push(reg("PCMSK0", 0x6b, "EXINT", "Pin Change Mask Register 0 (PB7..PB0)", (0..8).rev().map(|i| field(&format!("PCINT{i}"), 1 << i, "")).collect()));
    r.push(reg("PCMSK1", 0x6c, "EXINT", "Pin Change Mask Register 1 (PJ6..PJ0, PE0)", (8..16).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i - 8), "")).collect()));
    r.push(reg("PCMSK2", 0x6d, "EXINT", "Pin Change Mask Register 2 (PK7..PK0)", (16..24).rev().map(|i| field(&format!("PCINT{i}"), 1 << (i - 16), "")).collect()));
    r.push(reg("XMCRA", 0x74, "CPU", "External Memory Control Register A (external memory is not modelled)", vec![
        b("SRE", 0x80, "External SRAM/XMEM Enable"), b("SRL", 0x70, "Wait-state Sector Limit"), b("SRW1", 0x0c, "Wait-state Select bits for Upper Sector"), b("SRW0", 0x03, "Wait-state Select bits for Lower Sector"),
    ]));
    r.push(reg("XMCRB", 0x75, "CPU", "External Memory Control Register B (external memory is not modelled)", vec![
        b("XMBK", 0x80, "External Memory Bus-keeper Enable"), b("XMM", 0x07, "External Memory High Mask"),
    ]));
    r.extend(adc(true));
    r.push(reg("DIDR2", 0x7d, "ADC", "Digital Input Disable Register 2", (8..16).rev().map(|i| field(&format!("ADC{i}D"), 1 << (i - 8), "")).collect()));
    r.push(reg("DIDR0", 0x7e, "ADC", "Digital Input Disable Register 0", (0..8).rev().map(|i| field(&format!("ADC{i}D"), 1 << i, "")).collect()));
    r.extend(twi());
    for (k, base) in [(0, 0xc0), (1, 0xc8), (2, 0xd0), (3, 0x130)] {
        r.extend(usart(k, base));
    }
    r.sort_by_key(|x| x.addr);
    r
}

/// Table 14-1 "Reset and Interrupt Vectors" (57 vectors).
fn vector_names() -> Vec<String> {
    let mut n: Vec<String> = ["RESET", "INT0", "INT1", "INT2", "INT3", "INT4", "INT5", "INT6", "INT7", "PCINT0", "PCINT1", "PCINT2", "WDT", "TIMER2_COMPA", "TIMER2_COMPB", "TIMER2_OVF"].map(String::from).into();
    n.extend(timer_vectors(1, true));
    n.extend(["TIMER0_COMPA", "TIMER0_COMPB", "TIMER0_OVF", "SPI_STC"].map(String::from));
    n.extend(usart_vectors(0));
    n.extend(["ANALOG_COMP", "ADC", "EE_READY"].map(String::from));
    n.extend(timer_vectors(3, true));
    n.extend(usart_vectors(1));
    n.extend(["TWI", "SPM_READY"].map(String::from));
    n.extend(timer_vectors(4, true));
    n.extend(timer_vectors(5, true));
    n.extend(usart_vectors(2));
    n.extend(usart_vectors(3));
    n
}

/// TQFP-100 pinout (Atmel-2549Q figure 1-1 / table of pin functions).
fn pins() -> Vec<PinSpec> {
    let p = |n: u8, port: char, bit: u8, f: &[&str]| io(n, &format!("P{port}{bit}"), gpio(port, bit), f);
    let mut v = vec![
        p(1, 'G', 5, &["OC0B"]),
        p(2, 'E', 0, &["RXD0", "PCINT8"]),
        p(3, 'E', 1, &["TXD0"]),
        p(4, 'E', 2, &["XCK0", "AIN0"]),
        p(5, 'E', 3, &["OC3A", "AIN1"]),
        p(6, 'E', 4, &["OC3B", "INT4"]),
        p(7, 'E', 5, &["OC3C", "INT5"]),
        p(8, 'E', 6, &["T3", "INT6"]),
        p(9, 'E', 7, &["CLKO", "ICP3", "INT7"]),
        power(10, "VCC", PinKind::Vcc),
        power(11, "GND", PinKind::Gnd),
        p(12, 'H', 0, &["RXD2"]),
        p(13, 'H', 1, &["TXD2"]),
        p(14, 'H', 2, &["XCK2"]),
        p(15, 'H', 3, &["OC4A"]),
        p(16, 'H', 4, &["OC4B"]),
        p(17, 'H', 5, &["OC4C"]),
        p(18, 'H', 6, &["OC2B"]),
        p(19, 'B', 0, &["SS", "PCINT0"]),
        p(20, 'B', 1, &["SCK", "PCINT1"]),
        p(21, 'B', 2, &["MOSI", "PCINT2"]),
        p(22, 'B', 3, &["MISO", "PCINT3"]),
        p(23, 'B', 4, &["OC2A", "PCINT4"]),
        p(24, 'B', 5, &["OC1A", "PCINT5"]),
        p(25, 'B', 6, &["OC1B", "PCINT6"]),
        p(26, 'B', 7, &["OC0A", "OC1C", "PCINT7"]),
        p(27, 'H', 7, &["T4"]),
        p(28, 'G', 3, &["TOSC2"]),
        p(29, 'G', 4, &["TOSC1"]),
        dedicated(30, "RESET"),
        power(31, "VCC", PinKind::Vcc),
        power(32, "GND", PinKind::Gnd),
        dedicated(33, "XTAL2"),
        dedicated(34, "XTAL1"),
        p(35, 'L', 0, &["ICP4"]),
        p(36, 'L', 1, &["ICP5"]),
        p(37, 'L', 2, &["T5"]),
        p(38, 'L', 3, &["OC5A"]),
        p(39, 'L', 4, &["OC5B"]),
        p(40, 'L', 5, &["OC5C"]),
        p(41, 'L', 6, &[]),
        p(42, 'L', 7, &[]),
        p(43, 'D', 0, &["SCL", "INT0"]),
        p(44, 'D', 1, &["SDA", "INT1"]),
        p(45, 'D', 2, &["RXD1", "INT2"]),
        p(46, 'D', 3, &["TXD1", "INT3"]),
        p(47, 'D', 4, &["ICP1"]),
        p(48, 'D', 5, &["XCK1"]),
        p(49, 'D', 6, &["T1"]),
        p(50, 'D', 7, &["T0"]),
        p(51, 'G', 0, &["WR"]),
        p(52, 'G', 1, &["RD"]),
    ];
    for i in 0..8u8 {
        v.push(p(53 + i, 'C', i, &[&format!("A{}", 8 + i)]));
    }
    v.push(power(61, "VCC", PinKind::Vcc));
    v.push(power(62, "GND", PinKind::Gnd));
    for (i, f) in [(0u8, "RXD3"), (1, "TXD3"), (2, "XCK3")] {
        v.push(p(63 + i, 'J', i, &[f, &format!("PCINT{}", 9 + i)]));
    }
    for i in 3..7u8 {
        v.push(p(63 + i, 'J', i, &[&format!("PCINT{}", 9 + i)]));
    }
    v.push(p(70, 'G', 2, &["ALE"]));
    for i in 0..8u8 {
        let bit = 7 - i;
        v.push(p(71 + i, 'A', bit, &[&format!("AD{bit}")]));
    }
    v.push(p(79, 'J', 7, &[]));
    v.push(power(80, "VCC", PinKind::Vcc));
    v.push(power(81, "GND", PinKind::Gnd));
    for i in 0..8u8 {
        let bit = 7 - i;
        v.push(p(82 + i, 'K', bit, &[&format!("ADC{}", 8 + bit), &format!("PCINT{}", 16 + bit)]));
    }
    for i in 0..8u8 {
        let bit = 7 - i;
        let jtag = match bit {
            7 => Some("TDI"),
            6 => Some("TDO"),
            5 => Some("TMS"),
            4 => Some("TCK"),
            _ => None,
        };
        let adc = format!("ADC{bit}");
        let mut f = vec![adc.as_str()];
        f.extend(jtag);
        v.push(p(90 + i, 'F', bit, &f));
    }
    v.push(power(98, "AREF", PinKind::Ref));
    v.push(power(99, "GND", PinKind::Gnd));
    v.push(power(100, "AVCC", PinKind::Ref));
    v
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let mut groups: Vec<(String, String)> = vec![("CPU".into(), "CPU, Clock, Sleep, Reset & Power (and external memory control)".into())];
    groups.extend(PORTS.iter().map(|p| (format!("PORT{}", p.0), format!("I/O Port {}", p.0))));
    groups.push(("EXINT".into(), "External & Pin Change Interrupts".into()));
    for n in 0..=5 {
        let (bits, extra) = match n {
            0 | 2 => (8, if n == 2 { " (asynchronous mode not simulated)" } else { "" }),
            _ => (16, " and three compare units"),
        };
        groups.push((format!("TC{n}"), format!("{bits}-bit Timer/Counter{n} with PWM{extra}")));
    }
    groups.extend((0..4).map(|k| (format!("USART{k}"), format!("USART{k} (serial port)"))));
    groups.extend([
        ("SPI", "Serial Peripheral Interface"),
        ("TWI", "2-wire Serial Interface (I2C)"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter (16 channels)"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ].map(|(n, d)| (n.to_string(), d.to_string())));
    let big = v.flash > 131072;
    let mut features = feature::MOVW | feature::MUL | feature::LPMX | feature::SPM | feature::BREAK | feature::JMP;
    if v.flash > 65536 {
        features |= feature::ELPM | feature::ELPMX | feature::SPMX;
    }
    if big {
        features |= feature::EIJMP;
    }
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "megaAVR (ATmega640/1280/2560)".into(),
        core_name: match v.flash {
            65536 => "AVRe+ (AVR5)".into(),
            131072 => "AVRe+ (AVR51)".into(),
            _ => "AVRe+ (AVR6)".into(),
        },
        features,
        flash_size: v.flash,
        sram_start: SRAM_START,
        sram_size: SRAM,
        eeprom_size: 4096,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: v.signature,
        calibration: 0x9a,
        fuses: fuses(0x99),
        sleep: sleep(),
        // BOOTSZ1:0 = 00..11: 4096/2048/1024/512 words.
        boot: Some(BootSpec { sizes_words: [4096, 2048, 1024, 512] }),
        vectors: vectors(&vector_names()),
        registers: registers(v),
        groups: groups.into_iter().map(|(name, desc)| PeripheralGroupSpec { name, desc }).collect(),
        package: "TQFP-100".into(),
        pins: pins(),
        gpio_count: 86,
        has_adc: true,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // "Speed grades": 4 MHz @ 1.8 V, 8 MHz @ 2.7 V, 16 MHz @ 4.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (8e6, 2.7), (16e6, 4.5)],
        datasheet: "Atmel-2549Q ATmega640/1280/1281/2560/2561 (02/2014)".into(),
        die: None,
        peripheral_set: PeripheralSet::MegaX0,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
