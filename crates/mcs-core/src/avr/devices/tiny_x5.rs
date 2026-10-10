//! ATtiny25 / ATtiny45 / ATtiny85 (classic AVR core without MUL/JMP, PDIP-8).
//! Source: Atmel "ATtiny25/45/85" datasheet Atmel-2586Q (08/2013), register summary
//! (section 26), fuse tables (section 20.2) and avr-libc iotnx5.h.

use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash: u32,
    sram: u16,
    eeprom: u16,
    signature: [u8; 3],
}

const VARIANTS: &[Variant] = &[
    Variant { id: "attiny25", name: "ATtiny25", flash: 2048, sram: 128, eeprom: 128, signature: [0x1e, 0x91, 0x08] },
    Variant { id: "attiny45", name: "ATtiny45", flash: 4096, sram: 256, eeprom: 256, signature: [0x1e, 0x92, 0x06] },
    Variant { id: "attiny85", name: "ATtiny85", flash: 8192, sram: 512, eeprom: 512, signature: [0x1e, 0x93, 0x0b] },
];

pub(super) fn reg(name: &str, addr: u16, group: &str, desc: &str, bits: Vec<BitFieldSpec>) -> IoRegisterSpec {
    IoRegisterSpec { name: name.into(), addr, reset: 0, group: group.into(), desc: desc.into(), bits, access: RegisterAccess::Rw }
}

pub(super) fn b(name: &str, mask: u8, desc: &str) -> BitFieldSpec {
    field(name, mask, desc)
}

pub(super) fn nbits(prefix: &str, mask: u8) -> Vec<BitFieldSpec> {
    (0..8).rev().filter(|i| mask & (1 << i) != 0).map(|i| field(&format!("{prefix}{i}"), 1 << i, "")).collect()
}

fn registers(v: &Variant) -> Vec<IoRegisterSpec> {
    let ramend = 0x60 + v.sram - 1;
    let mut spl = reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]);
    spl.reset = (ramend & 0xff) as u8;
    let mut sph = reg("SPH", 0x5e, "CPU", "Stack Pointer High Byte", vec![]);
    sph.reset = (ramend >> 8) as u8;
    let mut adcl = reg("ADCL", 0x24, "ADC", "ADC Data Register Low Byte (read first)", vec![]);
    adcl.access = RegisterAccess::R;
    let mut adch = reg("ADCH", 0x25, "ADC", "ADC Data Register High Byte", vec![]);
    adch.access = RegisterAccess::R;
    let mut r = vec![
        reg("ADCSRB", 0x23, "ADC", "ADC Control and Status Register B", vec![b("BIN", 0x80, "Bipolar Input Mode"), b("ACME", 0x40, "Analog Comparator Multiplexer Enable"), b("IPR", 0x20, "Input Polarity Reversal"), b("ADTS", 0x07, "ADC Auto Trigger Source")]),
        adcl,
        adch,
        reg("ADCSRA", 0x26, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADMUX", 0x27, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS1", 0x80, "Reference Selection bit 1"), b("REFS0", 0x40, "Reference Selection bit 0"), b("ADLAR", 0x20, "ADC Left Adjust Result"),
            b("REFS2", 0x10, "Reference Selection bit 2 (000 VCC, 001 AREF, 010 1.1 V, 110 2.56 V)"), b("MUX", 0x0f, "Channel (0-3 ADCn, 4-11 differential, 12 1.1 V, 13 GND, 15 temperature)"),
        ]),
        reg("ACSR", 0x28, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("USICR", 0x2d, "USI", "USI Control Register", vec![
            b("USISIE", 0x80, "Start Condition Interrupt Enable"), b("USIOIE", 0x40, "Counter Overflow Interrupt Enable"), b("USIWM", 0x30, "Wire Mode (01 three-wire, 10 two-wire)"),
            b("USICS", 0x0c, "Clock Source Select"), b("USICLK", 0x02, "Clock Strobe"), b("USITC", 0x01, "Toggle Clock Port Pin"),
        ]),
        reg("USISR", 0x2e, "USI", "USI Status Register", vec![b("USISIF", 0x80, "Start Condition Interrupt Flag"), b("USIOIF", 0x40, "Counter Overflow Interrupt Flag"), b("USIPF", 0x20, "Stop Condition Flag"), b("USIDC", 0x10, "Data Output Collision"), b("USICNT", 0x0f, "Counter Value")]),
        reg("USIDR", 0x2f, "USI", "USI Data Register", vec![]),
        reg("USIBR", 0x30, "USI", "USI Buffer Register", vec![]),
        reg("GPIOR0", 0x31, "CPU", "General Purpose I/O Register 0", vec![]),
        reg("GPIOR1", 0x32, "CPU", "General Purpose I/O Register 1", vec![]),
        reg("GPIOR2", 0x33, "CPU", "General Purpose I/O Register 2", vec![]),
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
        reg("EEARL", 0x3e, "EEPROM", "EEPROM Address Register Low Byte", vec![]),
        reg("EEARH", 0x3f, "EEPROM", "EEPROM Address Register High Byte", vec![]),
        reg("PRR", 0x40, "CPU", "Power Reduction Register", vec![b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRTIM0", 0x04, "Power Reduction Timer/Counter0"), b("PRUSI", 0x02, "Power Reduction USI"), b("PRADC", 0x01, "Power Reduction ADC")]),
        reg("WDTCR", 0x41, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("DWDR", 0x42, "CPU", "debugWIRE Data Register", vec![]),
        reg("DTPS1", 0x43, "TC1", "Timer/Counter1 Dead Time Prescaler Register", vec![]),
        reg("DT1B", 0x44, "TC1", "Timer/Counter1 Dead Time B", vec![]),
        reg("DT1A", 0x45, "TC1", "Timer/Counter1 Dead Time A", vec![]),
        reg("CLKPR", 0x46, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("PLLCSR", 0x47, "TC1", "PLL Control and Status Register", vec![b("LSM", 0x80, "Low Speed Mode (32 MHz)"), b("PCKE", 0x04, "PCK Enable (Timer1 from the PLL)"), b("PLLE", 0x02, "PLL Enable"), b("PLOCK", 0x01, "PLL Lock Detector")]),
        reg("OCR0B", 0x48, "TC0", "Output Compare Register 0 B", vec![]),
        reg("OCR0A", 0x49, "TC0", "Output Compare Register 0 A", vec![]),
        reg("TCCR0A", 0x4a, "TC0", "Timer/Counter0 Control Register A", vec![b("COM0A", 0xc0, "Compare Output Mode A"), b("COM0B", 0x30, "Compare Output Mode B"), b("WGM01", 0x02, "Waveform Generation Mode bit 1"), b("WGM00", 0x01, "Waveform Generation Mode bit 0")]),
        reg("OCR1B", 0x4b, "TC1", "Output Compare Register 1 B", vec![]),
        reg("GTCCR", 0x4c, "TC1", "General Timer/Counter1 Control Register", vec![
            b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PWM1B", 0x40, "PWM Enable B"), b("COM1B", 0x30, "Compare Output Mode B"),
            b("FOC1B", 0x08, "Force Output Compare 1B"), b("FOC1A", 0x04, "Force Output Compare 1A"), b("PSR1", 0x02, "Prescaler Reset Timer/Counter1"), b("PSR0", 0x01, "Prescaler Reset Timer/Counter0"),
        ]),
        reg("OCR1C", 0x4d, "TC1", "Output Compare Register 1 C (TOP)", vec![]),
        reg("OCR1A", 0x4e, "TC1", "Output Compare Register 1 A", vec![]),
        reg("TCNT1", 0x4f, "TC1", "Timer/Counter1", vec![]),
        reg("TCCR1", 0x50, "TC1", "Timer/Counter1 Control Register", vec![
            b("CTC1", 0x80, "Clear Timer on Compare Match (OCR1C)"), b("PWM1A", 0x40, "PWM Enable A"), b("COM1A", 0x30, "Compare Output Mode A"),
            b("CS1", 0x0f, "Clock Select (0 stop, n = CK or PCK / 2^(n-1))"),
        ]),
        reg("OSCCAL", 0x51, "CPU", "Oscillator Calibration Register", vec![]),
        reg("TCNT0", 0x52, "TC0", "Timer/Counter0", vec![]),
        reg("TCCR0B", 0x53, "TC0", "Timer/Counter0 Control Register B", vec![b("FOC0A", 0x80, "Force Output Compare A"), b("FOC0B", 0x40, "Force Output Compare B"), b("WGM02", 0x08, "Waveform Generation Mode bit 2"), b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("BODS", 0x80, "BOD Sleep"), b("PUD", 0x40, "Pull-up Disable"), b("SE", 0x20, "Sleep Enable"), b("SM", 0x18, "Sleep Mode (00 idle, 01 ADC NR, 10 power-down)"),
            b("BODSE", 0x04, "BOD Sleep Enable"), b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)"),
        ]),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![b("RSIG", 0x20, "Read Device Signature Imprint Table"), b("CTPB", 0x10, "Clear Temporary Page Buffer"), b("RFLB", 0x08, "Read Fuse and Lock Bits"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Self Programming Enable")]),
        reg("TIFR", 0x58, "TC0", "Timer/Counter Interrupt Flag Register", vec![
            b("OCF1A", 0x40, "Timer1 Output Compare A Flag"), b("OCF1B", 0x20, "Timer1 Output Compare B Flag"), b("OCF0A", 0x10, "Timer0 Output Compare A Flag"),
            b("OCF0B", 0x08, "Timer0 Output Compare B Flag"), b("TOV1", 0x04, "Timer1 Overflow Flag"), b("TOV0", 0x02, "Timer0 Overflow Flag"),
        ]),
        reg("TIMSK", 0x59, "TC0", "Timer/Counter Interrupt Mask Register", vec![
            b("OCIE1A", 0x40, "Timer1 Output Compare A Interrupt Enable"), b("OCIE1B", 0x20, "Timer1 Output Compare B Interrupt Enable"), b("OCIE0A", 0x10, "Timer0 Output Compare A Interrupt Enable"),
            b("OCIE0B", 0x08, "Timer0 Output Compare B Interrupt Enable"), b("TOIE1", 0x04, "Timer1 Overflow Interrupt Enable"), b("TOIE0", 0x02, "Timer0 Overflow Interrupt Enable"),
        ]),
        reg("GIFR", 0x5a, "EXINT", "General Interrupt Flag Register", vec![b("INTF0", 0x40, "External Interrupt Flag 0"), b("PCIF", 0x20, "Pin Change Interrupt Flag")]),
        reg("GIMSK", 0x5b, "EXINT", "General Interrupt Mask Register", vec![b("INT0", 0x40, "External Interrupt Request 0 Enable"), b("PCIE", 0x20, "Pin Change Interrupt Enable")]),
        spl,
        sph,
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ];
    r.sort_by_key(|x| x.addr);
    r
}

const VECTOR_NAMES: [(&str, &str); 15] = [
    ("RESET", "External Pin, Power-on Reset, Brown-out Reset, Watchdog Reset"),
    ("INT0", "External Interrupt Request 0"),
    ("PCINT0", "Pin Change Interrupt Request 0"),
    ("TIM1_COMPA", "Timer/Counter1 Compare Match A"),
    ("TIM1_OVF", "Timer/Counter1 Overflow"),
    ("TIM0_OVF", "Timer/Counter0 Overflow"),
    ("EE_RDY", "EEPROM Ready"),
    ("ANA_COMP", "Analog Comparator"),
    ("ADC", "ADC Conversion Complete"),
    ("TIM1_COMPB", "Timer/Counter1 Compare Match B"),
    ("TIM0_COMPA", "Timer/Counter0 Compare Match A"),
    ("TIM0_COMPB", "Timer/Counter0 Compare Match B"),
    ("WDT", "Watchdog Time-out"),
    ("USI_START", "USI START"),
    ("USI_OVF", "USI Overflow"),
];

fn fuses() -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    vec![
        FuseByteSpec {
            name: "Low".into(),
            default: 0x62,
            bits: vec![
                f("CKDIV8", 0x80, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
                f("CKOUT", 0x40, "Clock output on PB4 (CLKO) when programmed (0)"),
                f("SUT", 0x30, "Start-up time select"),
                f("CKSEL", 0x0f, "Clock source (0000 external clock, 0001 PLL 16 MHz, 0010 internal 8 MHz RC, 0011 ATtiny15 mode, 0100 internal 128 kHz, 0110 32 kHz crystal, 1000-1111 crystal)"),
            ],
        },
        FuseByteSpec {
            name: "High".into(),
            default: 0xdf,
            bits: vec![
                f("RSTDISBL", 0x80, "External reset disabled (PB5 becomes I/O) when programmed (0)"),
                f("DWEN", 0x40, "debugWIRE enabled when programmed (0)"),
                f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
                f("WDTON", 0x10, "Watchdog Timer always on when programmed (0)"),
                f("EESAVE", 0x08, "EEPROM preserved through chip erase when programmed (0)"),
                f("BODLEVEL", 0x07, "Brown-out detector level (111 disabled, 110 1.8 V, 101 2.7 V, 100 4.3 V)"),
            ],
        },
        FuseByteSpec { name: "Extended".into(), default: 0xff, bits: vec![f("SELFPRGEN", 0x01, "Self-programming enabled when programmed (0)")] },
    ]
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let io = |number: u8, name: &str, gpio: u8, functions: Vec<String>| PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions };
    let pins = vec![
        io(1, "PB5", 5, s(&["RESET", "ADC0", "dW", "PCINT5"])),
        io(2, "PB3", 3, s(&["XTAL1", "CLKI", "OC1B-bar", "ADC3", "PCINT3"])),
        io(3, "PB4", 4, s(&["XTAL2", "CLKO", "OC1B", "ADC2", "PCINT4"])),
        PinSpec { number: 4, name: "GND".into(), kind: PinKind::Gnd, gpio: None, functions: vec![] },
        io(5, "PB0", 0, s(&["MOSI", "DI", "SDA", "AIN0", "OC0A", "OC1A-bar", "AREF", "PCINT0"])),
        io(6, "PB1", 1, s(&["MISO", "DO", "AIN1", "OC0B", "OC1A", "PCINT1"])),
        io(7, "PB2", 2, s(&["SCK", "USCK", "SCL", "ADC1", "T0", "INT0", "PCINT2"])),
        PinSpec { number: 8, name: "VCC".into(), kind: PinKind::Vcc, gpio: None, functions: vec![] },
    ];
    let groups = [
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTB", "I/O Port B"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("TC1", "8-bit high-speed Timer/Counter1 (PLL)"),
        ("USI", "Universal Serial Interface"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "tinyAVR (ATtiny25/45/85)".into(),
        core_name: "AVRe (AVR25)".into(),
        features: feature::MOVW | feature::LPMX | feature::SPM | feature::BREAK,
        flash_size: v.flash,
        sram_start: 0x60,
        sram_size: v.sram,
        eeprom_size: v.eeprom,
        io_base: 0x20,
        io_size: 64,
        regs_in_data_space: true,
        flash_map_base: None,
        nvm_map: None,
        signature: v.signature,
        calibration: 0x8a,
        fuses: fuses(),
        // Section 7.5.2 MCUCR: SM1:0 = 00 idle, 01 ADC NR, 10 power-down.
        sleep: SleepControl {
            register: "MCUCR".into(),
            se_mask: 0x20,
            sm_mask: 0x18,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown)],
        },
        boot: None,
        vectors: VECTOR_NAMES.iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-8".into(),
        pins,
        gpio_count: 6,
        has_adc: true,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Section 21.3 "Speed": 10 MHz @ 2.7-5.5 V, 20 MHz @ 4.5-5.5 V (ATtiny85V: 4 MHz @ 1.8 V).
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "ATtiny25/45/85 datasheet Atmel-2586Q (08/2013)".into(),
        die: None,
        peripheral_set: PeripheralSet::TinyX5,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
