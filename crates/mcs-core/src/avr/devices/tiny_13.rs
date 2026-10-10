//! ATtiny13A (classic AVR core, 1 KB flash, 64 B SRAM, 64 B EEPROM, PDIP-8).
//! Source: Atmel "ATtiny13A" datasheet Atmel-8126F (05/2012): register summary,
//! fuse tables, clock system; register addresses and bit positions
//! cross-checked against avr-libc iotn13a.h.
//!
//! Differences from the ATtiny25/45/85: a single 8-bit Timer/Counter0 (no Timer1, no USI), a
//! 4-channel single-ended 10-bit ADC (REFS0 selects VCC / 1.1 V; no differential channels, no
//! temperature sensor), the internal RC oscillator is 9.6 MHz (CKSEL 10) or 4.8 MHz (CKSEL 01),
//! the fuses are SPIEN/EESAVE/WDTON/CKDIV8/SUT/CKSEL (low) and SELFPRGEN/DWEN/BODLEVEL1:0/
//! RSTDISBL (high), and BODS/BODSE live in their own BODCR register.
//!
//! Deliberate simplifications: SPM self-programming, debugWIRE (DWDR), the BOD-disable-in-sleep
//! sequence (BODCR is plain storage) and the 4.8 MHz / 9.6 MHz speed-versus-VCC derating are not
//! modelled.

use super::tiny_x5::{b, nbits, reg};
use crate::avr::device::*;
use crate::avr::isa::feature;

fn registers() -> Vec<IoRegisterSpec> {
    let ramend = 0x60 + 64 - 1;
    let mut spl = reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]);
    spl.reset = ramend as u8;
    let mut adcl = reg("ADCL", 0x24, "ADC", "ADC Data Register Low Byte (read first)", vec![]);
    adcl.access = RegisterAccess::R;
    let mut adch = reg("ADCH", 0x25, "ADC", "ADC Data Register High Byte", vec![]);
    adch.access = RegisterAccess::R;
    let mut r = vec![
        reg("ADCSRB", 0x23, "ADC", "ADC Control and Status Register B", vec![b("ACME", 0x40, "Analog Comparator Multiplexer Enable"), b("ADTS", 0x07, "ADC Auto Trigger Source (0 free running, 1 AC, 2 INT0, 3 T0 compare A, 4 T0 overflow, 5 T0 compare B, 6 pin change)")]),
        adcl,
        adch,
        reg("ADCSRA", 0x26, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADMUX", 0x27, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS0", 0x40, "Reference Selection (0 VCC, 1 internal 1.1 V)"), b("ADLAR", 0x20, "ADC Left Adjust Result"), b("MUX", 0x03, "Channel (0 ADC0/PB5, 1 ADC1/PB2, 2 ADC2/PB4, 3 ADC3/PB3)"),
        ]),
        reg("ACSR", 0x28, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("DIDR0", 0x34, "ADC", "Digital Input Disable Register 0", vec![b("ADC0D", 0x20, ""), b("ADC2D", 0x10, ""), b("ADC3D", 0x08, ""), b("ADC1D", 0x04, ""), b("AIN1D", 0x02, ""), b("AIN0D", 0x01, "")]),
        reg("PCMSK", 0x35, "EXINT", "Pin Change Mask Register", nbits("PCINT", 0x3f)),
        reg("PINB", 0x36, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", nbits("PINB", 0x3f)),
        reg("DDRB", 0x37, "PORTB", "Port B Data Direction Register", nbits("DDB", 0x3f)),
        reg("PORTB", 0x38, "PORTB", "Port B Data Register", nbits("PORTB", 0x3f)),
        reg("EECR", 0x3c, "EEPROM", "EEPROM Control Register", vec![
            b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
            b("EEMPE", 0x04, "EEPROM Master Program Enable"), b("EEPE", 0x02, "EEPROM Program Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x3d, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEARL", 0x3e, "EEPROM", "EEPROM Address Register (6 bits)", vec![]),
        reg("WDTCR", 0x41, "WDT", "Watchdog Timer Control Register", vec![
            b("WDTIF", 0x80, "Watchdog Timeout Interrupt Flag"), b("WDTIE", 0x40, "Watchdog Timeout Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("PRR", 0x45, "CPU", "Power Reduction Register", vec![b("PRTIM0", 0x02, "Power Reduction Timer/Counter0"), b("PRADC", 0x01, "Power Reduction ADC")]),
        reg("CLKPR", 0x46, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("GTCCR", 0x48, "TC0", "General Timer/Counter Control Register", vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PSR10", 0x01, "Prescaler Reset Timer/Counter0")]),
        reg("OCR0B", 0x49, "TC0", "Output Compare Register 0 B", vec![]),
        reg("DWDR", 0x4e, "CPU", "debugWIRE Data Register", vec![]),
        reg("TCCR0A", 0x4f, "TC0", "Timer/Counter0 Control Register A", vec![b("COM0A", 0xc0, "Compare Output Mode A"), b("COM0B", 0x30, "Compare Output Mode B"), b("WGM01", 0x02, "Waveform Generation Mode bit 1"), b("WGM00", 0x01, "Waveform Generation Mode bit 0")]),
        reg("BODCR", 0x50, "CPU", "Brown-Out Detector Control Register", vec![b("BODS", 0x02, "BOD Sleep"), b("BODSE", 0x01, "BOD Sleep Enable")]),
        reg("OSCCAL", 0x51, "CPU", "Oscillator Calibration Register", vec![]),
        reg("TCNT0", 0x52, "TC0", "Timer/Counter0", vec![]),
        reg("TCCR0B", 0x53, "TC0", "Timer/Counter0 Control Register B", vec![b("FOC0A", 0x80, "Force Output Compare A"), b("FOC0B", 0x40, "Force Output Compare B"), b("WGM02", 0x08, "Waveform Generation Mode bit 2"), b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("PUD", 0x40, "Pull-up Disable"), b("SE", 0x20, "Sleep Enable"), b("SM", 0x18, "Sleep Mode (00 idle, 01 ADC NR, 10 power-down)"),
            b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)"),
        ]),
        reg("OCR0A", 0x56, "TC0", "Output Compare Register 0 A", vec![]),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![b("CTPB", 0x10, "Clear Temporary Page Buffer"), b("RFLB", 0x08, "Read Fuse and Lock Bits"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Self Programming Enable")]),
        reg("TIFR0", 0x58, "TC0", "Timer/Counter0 Interrupt Flag Register", vec![b("OCF0B", 0x08, "Timer0 Output Compare B Flag"), b("OCF0A", 0x04, "Timer0 Output Compare A Flag"), b("TOV0", 0x02, "Timer0 Overflow Flag")]),
        reg("TIMSK0", 0x59, "TC0", "Timer/Counter0 Interrupt Mask Register", vec![b("OCIE0B", 0x08, "Timer0 Output Compare B Interrupt Enable"), b("OCIE0A", 0x04, "Timer0 Output Compare A Interrupt Enable"), b("TOIE0", 0x02, "Timer0 Overflow Interrupt Enable")]),
        reg("GIFR", 0x5a, "EXINT", "General Interrupt Flag Register", vec![b("INTF0", 0x40, "External Interrupt Flag 0"), b("PCIF", 0x20, "Pin Change Interrupt Flag")]),
        reg("GIMSK", 0x5b, "EXINT", "General Interrupt Mask Register", vec![b("INT0", 0x40, "External Interrupt Request 0 Enable"), b("PCIE", 0x20, "Pin Change Interrupt Enable")]),
        spl,
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ];
    r.sort_by_key(|x| x.addr);
    r
}

const VECTOR_NAMES: [(&str, &str); 10] = [
    ("RESET", "External Pin, Power-on Reset, Brown-out Reset, Watchdog Reset"),
    ("INT0", "External Interrupt Request 0"),
    ("PCINT0", "Pin Change Interrupt Request 0"),
    ("TIM0_OVF", "Timer/Counter0 Overflow"),
    ("EE_RDY", "EEPROM Ready"),
    ("ANA_COMP", "Analog Comparator"),
    ("TIM0_COMPA", "Timer/Counter0 Compare Match A"),
    ("TIM0_COMPB", "Timer/Counter0 Compare Match B"),
    ("WDT", "Watchdog Time-out"),
    ("ADC", "ADC Conversion Complete"),
];

fn fuses() -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    vec![
        FuseByteSpec {
            name: "Low".into(),
            default: 0x6a,
            bits: vec![
                f("SPIEN", 0x80, "Serial programming enabled when programmed (0)"),
                f("EESAVE", 0x40, "EEPROM preserved through chip erase when programmed (0)"),
                f("WDTON", 0x20, "Watchdog Timer always on when programmed (0)"),
                f("CKDIV8", 0x10, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
                f("SUT", 0x0c, "Start-up time select"),
                f("CKSEL", 0x03, "Clock source (00 external clock, 01 internal 4.8 MHz, 10 internal 9.6 MHz, 11 internal 128 kHz)"),
            ],
        },
        FuseByteSpec {
            name: "High".into(),
            default: 0xff,
            bits: vec![
                f("SELFPRGEN", 0x10, "Self-programming enabled when programmed (0)"),
                f("DWEN", 0x08, "debugWIRE enabled when programmed (0)"),
                f("BODLEVEL", 0x06, "Brown-out detector level (11 disabled, 10 1.8 V, 01 2.7 V, 00 4.3 V)"),
                f("RSTDISBL", 0x01, "External reset disabled (PB5 becomes I/O) when programmed (0)"),
            ],
        },
    ]
}

fn spec() -> AvrDeviceSpec {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let io = |number: u8, name: &str, gpio: u8, functions: Vec<String>| PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions };
    let pins = vec![
        io(1, "PB5", 5, s(&["RESET", "ADC0", "dW", "PCINT5"])),
        io(2, "PB3", 3, s(&["XTAL1", "CLKI", "ADC3", "PCINT3"])),
        io(3, "PB4", 4, s(&["XTAL2", "ADC2", "PCINT4"])),
        PinSpec { number: 4, name: "GND".into(), kind: PinKind::Gnd, gpio: None, functions: vec![] },
        io(5, "PB0", 0, s(&["MOSI", "AIN0", "OC0A", "PCINT0"])),
        io(6, "PB1", 1, s(&["MISO", "AIN1", "OC0B", "INT0", "PCINT1"])),
        io(7, "PB2", 2, s(&["SCK", "ADC1", "T0", "PCINT2"])),
        PinSpec { number: 8, name: "VCC".into(), kind: PinKind::Vcc, gpio: None, functions: vec![] },
    ];
    let groups = [
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTB", "I/O Port B"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    AvrDeviceSpec {
        id: "attiny13a".into(),
        name: "ATtiny13A".into(),
        family: "tinyAVR (ATtiny13A)".into(),
        core_name: "AVRe (AVR25)".into(),
        features: feature::MOVW | feature::LPMX | feature::SPM | feature::BREAK,
        flash_size: 1024,
        sram_start: 0x60,
        sram_size: 64,
        eeprom_size: 64,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: [0x1e, 0x90, 0x07],
        calibration: 0x5d, // nominal: the real value is per chip
        fuses: fuses(),
        // MCUCR: SM1:0 = 00 idle, 01 ADC NR, 10 power-down.
        sleep: SleepControl {
            register: "MCUCR".into(),
            se_mask: 0x20,
            sm_mask: 0x18,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown)],
        },
        boot: None,
        vectors: VECTOR_NAMES.iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-8".into(),
        pins,
        gpio_count: 6,
        has_adc: true,
        // Clock system: 9.6 MHz internal RC (CKSEL 10, 4.8 MHz with CKSEL 01), 128 kHz watchdog oscillator,
        // CKDIV8 programmed at delivery (1.2 MHz).
        clock: ClockSpec { internal_hz: 9_600_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Speed grades: 4 MHz @ 1.8-5.5 V, 10 MHz @ 2.7-5.5 V, 20 MHz @ 4.5-5.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "ATtiny13A datasheet Atmel-8126F (05/2012)".into(),
        die: None,
        peripheral_set: PeripheralSet::Tiny13,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    vec![spec()]
}
