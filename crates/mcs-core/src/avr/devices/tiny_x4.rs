//! ATtiny24A / ATtiny44A / ATtiny84A (classic AVR core without MUL/JMP, PDIP-14).
//! Source: Atmel "ATtiny24A/44A/84A" datasheet Atmel-8183F (06/2012): register summary, fuse
//! tables, clock system, ADC channel tables (single-ended, 20 differential pairs at 1x/20x with
//! polarity reversal, offset calibration, 1.1 V, GND, temperature); register addresses and bit
//! positions cross-checked against avr-libc iotn84a.h / iotnx4.h and the ATtiny24/44/84 register
//! summary (Atmel-7701G).
//!
//! Differences from the ATtiny25/45/85: two GPIO ports (PA0-7, PB0-3), a 16-bit Timer/Counter1
//! with input capture next to the 8-bit Timer/Counter0 (separate TIFRn/TIMSKn registers),
//! 8 single-ended ADC channels with the ADLAR bit in ADCSRB, two pin-change groups
//! (PCINT7:0 on port A, PCINT11:8 on port B), standby sleep mode, the USI on PA6/PA5/PA4.
//!
//! Deliberate simplifications: SPM self-programming, debugWIRE (DWDR), the BOD-disable-in-sleep
//! sequence (BODS/BODSE are plain MCUCR bits), the ADC offset-calibration channels (they read
//! as the differential pair with equal inputs, i.e. 0) and the speed-versus-VCC derating
//! curve beyond the three speed grades are not modelled.

use super::tiny_x5::{b, nbits, reg};
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
    Variant { id: "attiny24a", name: "ATtiny24A", flash: 2048, sram: 128, eeprom: 128, signature: [0x1e, 0x91, 0x0b] },
    Variant { id: "attiny44a", name: "ATtiny44A", flash: 4096, sram: 256, eeprom: 256, signature: [0x1e, 0x92, 0x07] },
    Variant { id: "attiny84a", name: "ATtiny84A", flash: 8192, sram: 512, eeprom: 512, signature: [0x1e, 0x93, 0x0c] },
];

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
    let r16 = |name: &str, addr: u16, group: &str, desc: &str| vec![reg(&format!("{name}L"), addr, group, &format!("{desc} Low Byte"), vec![]), reg(&format!("{name}H"), addr + 1, group, &format!("{desc} High Byte"), vec![])];
    let mut r = vec![
        reg("PRR", 0x20, "CPU", "Power Reduction Register", vec![b("PRTIM1", 0x08, "Power Reduction Timer/Counter1"), b("PRTIM0", 0x04, "Power Reduction Timer/Counter0"), b("PRUSI", 0x02, "Power Reduction USI"), b("PRADC", 0x01, "Power Reduction ADC")]),
        reg("DIDR0", 0x21, "ADC", "Digital Input Disable Register 0", (0..8).rev().map(|i| b(&format!("ADC{i}D"), 1 << i, "")).collect()),
        reg("ADCSRB", 0x23, "ADC", "ADC Control and Status Register B", vec![
            b("BIN", 0x80, "Bipolar Input Mode"), b("ACME", 0x40, "Analog Comparator Multiplexer Enable"), b("ADLAR", 0x10, "ADC Left Adjust Result"),
            b("ADTS", 0x07, "ADC Auto Trigger Source (0 free running, 1 AC, 2 INT0, 3 T0 compare A, 4 T0 overflow, 5 T1 compare B, 6 T1 overflow, 7 T1 capture)"),
        ]),
        adcl,
        adch,
        reg("ADCSRA", 0x26, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADMUX", 0x27, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS", 0xc0, "Reference Selection (00 VCC, 01 AREF on PA0, 10 internal 1.1 V)"),
            b("MUX", 0x3f, "Channel (0-7 ADCn, 8-31 and 40-63 differential pairs, MUX0 selects 20x gain, 32 GND, 33 1.1 V, 34 temperature)"),
        ]),
        reg("ACSR", 0x28, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Analog Comparator Input Capture Enable"),
            b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("TIFR1", 0x2b, "TC1", "Timer/Counter1 Interrupt Flag Register", vec![b("ICF1", 0x20, "Timer1 Input Capture Flag"), b("OCF1B", 0x04, "Timer1 Output Compare B Flag"), b("OCF1A", 0x02, "Timer1 Output Compare A Flag"), b("TOV1", 0x01, "Timer1 Overflow Flag")]),
        reg("TIMSK1", 0x2c, "TC1", "Timer/Counter1 Interrupt Mask Register", vec![b("ICIE1", 0x20, "Timer1 Input Capture Interrupt Enable"), b("OCIE1B", 0x04, "Timer1 Output Compare B Interrupt Enable"), b("OCIE1A", 0x02, "Timer1 Output Compare A Interrupt Enable"), b("TOIE1", 0x01, "Timer1 Overflow Interrupt Enable")]),
        reg("USICR", 0x2d, "USI", "USI Control Register", vec![
            b("USISIE", 0x80, "Start Condition Interrupt Enable"), b("USIOIE", 0x40, "Counter Overflow Interrupt Enable"), b("USIWM", 0x30, "Wire Mode (01 three-wire, 10 two-wire)"),
            b("USICS", 0x0c, "Clock Source Select"), b("USICLK", 0x02, "Clock Strobe"), b("USITC", 0x01, "Toggle Clock Port Pin"),
        ]),
        reg("USISR", 0x2e, "USI", "USI Status Register", vec![b("USISIF", 0x80, "Start Condition Interrupt Flag"), b("USIOIF", 0x40, "Counter Overflow Interrupt Flag"), b("USIPF", 0x20, "Stop Condition Flag"), b("USIDC", 0x10, "Data Output Collision"), b("USICNT", 0x0f, "Counter Value")]),
        reg("USIDR", 0x2f, "USI", "USI Data Register", vec![]),
        reg("USIBR", 0x30, "USI", "USI Buffer Register", vec![]),
        reg("PCMSK0", 0x32, "EXINT", "Pin Change Mask Register 0 (PA7:0)", nbits("PCINT", 0xff)),
        reg("GPIOR0", 0x33, "CPU", "General Purpose I/O Register 0", vec![]),
        reg("GPIOR1", 0x34, "CPU", "General Purpose I/O Register 1", vec![]),
        reg("GPIOR2", 0x35, "CPU", "General Purpose I/O Register 2", vec![]),
        reg("PINB", 0x36, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", nbits("PINB", 0x0f)),
        reg("DDRB", 0x37, "PORTB", "Port B Data Direction Register", nbits("DDB", 0x0f)),
        reg("PORTB", 0x38, "PORTB", "Port B Data Register", nbits("PORTB", 0x0f)),
        reg("PINA", 0x39, "PORTA", "Port A Input Pins (write 1 toggles PORTA bit)", nbits("PINA", 0xff)),
        reg("DDRA", 0x3a, "PORTA", "Port A Data Direction Register", nbits("DDA", 0xff)),
        reg("PORTA", 0x3b, "PORTA", "Port A Data Register", nbits("PORTA", 0xff)),
        reg("EECR", 0x3c, "EEPROM", "EEPROM Control Register", vec![
            b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
            b("EEMPE", 0x04, "EEPROM Master Program Enable"), b("EEPE", 0x02, "EEPROM Program Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x3d, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEARL", 0x3e, "EEPROM", "EEPROM Address Register Low Byte", vec![]),
        reg("EEARH", 0x3f, "EEPROM", "EEPROM Address Register High Byte", vec![b("EEAR8", 0x01, "EEPROM Address bit 8")]),
        reg("PCMSK1", 0x40, "EXINT", "Pin Change Mask Register 1 (PB3:0)", (0..4).rev().map(|i| b(&format!("PCINT{}", 8 + i), 1 << i, "")).collect()),
        reg("WDTCSR", 0x41, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("TCCR1C", 0x42, "TC1", "Timer/Counter1 Control Register C", vec![b("FOC1A", 0x80, "Force Output Compare for Channel A"), b("FOC1B", 0x40, "Force Output Compare for Channel B")]),
        reg("GTCCR", 0x43, "TC1", "General Timer/Counter Control Register", vec![b("TSM", 0x80, "Timer/Counter Synchronization Mode"), b("PSR10", 0x01, "Prescaler Reset Timer/Counter1 and Timer/Counter0")]),
        reg("CLKPR", 0x46, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("DWDR", 0x47, "CPU", "debugWIRE Data Register", vec![]),
        reg("TCCR1B", 0x4e, "TC1", "Timer/Counter1 Control Register B", vec![
            b("ICNC1", 0x80, "Input Capture Noise Canceler"), b("ICES1", 0x40, "Input Capture Edge Select"), b("WGM13", 0x10, "Waveform Generation Mode bit 3"), b("WGM12", 0x08, "Waveform Generation Mode bit 2"),
            b("CS1", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T1 falling, 7 T1 rising)"),
        ]),
        reg("TCCR1A", 0x4f, "TC1", "Timer/Counter1 Control Register A", vec![b("COM1A", 0xc0, "Compare Output Mode A"), b("COM1B", 0x30, "Compare Output Mode B"), b("WGM11", 0x02, "Waveform Generation Mode bit 1"), b("WGM10", 0x01, "Waveform Generation Mode bit 0")]),
        reg("TCCR0A", 0x50, "TC0", "Timer/Counter0 Control Register A", vec![b("COM0A", 0xc0, "Compare Output Mode A"), b("COM0B", 0x30, "Compare Output Mode B"), b("WGM01", 0x02, "Waveform Generation Mode bit 1"), b("WGM00", 0x01, "Waveform Generation Mode bit 0")]),
        reg("OSCCAL", 0x51, "CPU", "Oscillator Calibration Register", vec![]),
        reg("TCNT0", 0x52, "TC0", "Timer/Counter0", vec![]),
        reg("TCCR0B", 0x53, "TC0", "Timer/Counter0 Control Register B", vec![b("FOC0A", 0x80, "Force Output Compare A"), b("FOC0B", 0x40, "Force Output Compare B"), b("WGM02", 0x08, "Waveform Generation Mode bit 2"), b("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"), b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag")]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", vec![
            b("BODS", 0x80, "BOD Sleep"), b("PUD", 0x40, "Pull-up Disable"), b("SE", 0x20, "Sleep Enable"), b("SM", 0x18, "Sleep Mode (00 idle, 01 ADC NR, 10 power-down, 11 standby)"),
            b("BODSE", 0x04, "BOD Sleep Enable"), b("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)"),
        ]),
        reg("OCR0A", 0x56, "TC0", "Output Compare Register 0 A", vec![]),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![b("CTPB", 0x10, "Clear Temporary Page Buffer"), b("RFLB", 0x08, "Read Fuse and Lock Bits"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Self Programming Enable")]),
        reg("TIFR0", 0x58, "TC0", "Timer/Counter0 Interrupt Flag Register", vec![b("OCF0B", 0x04, "Timer0 Output Compare B Flag"), b("OCF0A", 0x02, "Timer0 Output Compare A Flag"), b("TOV0", 0x01, "Timer0 Overflow Flag")]),
        reg("TIMSK0", 0x59, "TC0", "Timer/Counter0 Interrupt Mask Register", vec![b("OCIE0B", 0x04, "Timer0 Output Compare B Interrupt Enable"), b("OCIE0A", 0x02, "Timer0 Output Compare A Interrupt Enable"), b("TOIE0", 0x01, "Timer0 Overflow Interrupt Enable")]),
        reg("GIFR", 0x5a, "EXINT", "General Interrupt Flag Register", vec![b("INTF0", 0x40, "External Interrupt Flag 0"), b("PCIF1", 0x20, "Pin Change Interrupt Flag 1"), b("PCIF0", 0x10, "Pin Change Interrupt Flag 0")]),
        reg("GIMSK", 0x5b, "EXINT", "General Interrupt Mask Register", vec![b("INT0", 0x40, "External Interrupt Request 0 Enable"), b("PCIE1", 0x20, "Pin Change Interrupt Enable 1"), b("PCIE0", 0x10, "Pin Change Interrupt Enable 0")]),
        reg("OCR0B", 0x5c, "TC0", "Output Compare Register 0 B", vec![]),
        spl,
        sph,
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ];
    r.extend(r16("ICR1", 0x44, "TC1", "Timer/Counter1 Input Capture Register"));
    r.extend(r16("OCR1B", 0x48, "TC1", "Timer/Counter1 Output Compare Register B"));
    r.extend(r16("OCR1A", 0x4a, "TC1", "Timer/Counter1 Output Compare Register A"));
    r.extend(r16("TCNT1", 0x4c, "TC1", "Timer/Counter1"));
    r.sort_by_key(|x| x.addr);
    r
}

const VECTOR_NAMES: [(&str, &str); 17] = [
    ("RESET", "External Pin, Power-on Reset, Brown-out Reset, Watchdog Reset"),
    ("INT0", "External Interrupt Request 0"),
    ("PCINT0", "Pin Change Interrupt Request 0 (PA7:0)"),
    ("PCINT1", "Pin Change Interrupt Request 1 (PB3:0)"),
    ("WDT", "Watchdog Time-out"),
    ("TIM1_CAPT", "Timer/Counter1 Input Capture"),
    ("TIM1_COMPA", "Timer/Counter1 Compare Match A"),
    ("TIM1_COMPB", "Timer/Counter1 Compare Match B"),
    ("TIM1_OVF", "Timer/Counter1 Overflow"),
    ("TIM0_COMPA", "Timer/Counter0 Compare Match A"),
    ("TIM0_COMPB", "Timer/Counter0 Compare Match B"),
    ("TIM0_OVF", "Timer/Counter0 Overflow"),
    ("ANA_COMP", "Analog Comparator"),
    ("ADC", "ADC Conversion Complete"),
    ("EE_RDY", "EEPROM Ready"),
    ("USI_STR", "USI START"),
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
                f("CKOUT", 0x40, "Clock output on PB2 (CKOUT) when programmed (0)"),
                f("SUT", 0x30, "Start-up time select"),
                f("CKSEL", 0x0f, "Clock source (0000 external clock, 0010 internal 8 MHz, 0100 internal 128 kHz, 0110 external 32 kHz oscillator, 1000-1111 crystal)"),
            ],
        },
        FuseByteSpec {
            name: "High".into(),
            default: 0xdf,
            bits: vec![
                f("RSTDISBL", 0x80, "External reset disabled (PB3 becomes I/O) when programmed (0)"),
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
    let power = |number: u8, name: &str, kind: PinKind| PinSpec { number, name: name.into(), kind, gpio: None, functions: vec![] };
    // GPIO numbering: PA0-7 = 0-7, PB0-3 = 8-11.
    let pins = vec![
        power(1, "VCC", PinKind::Vcc),
        io(2, "PB0", 8, s(&["XTAL1", "CLKI", "PCINT8"])),
        io(3, "PB1", 9, s(&["XTAL2", "PCINT9"])),
        io(4, "PB3", 11, s(&["RESET", "dW", "PCINT11"])),
        io(5, "PB2", 10, s(&["INT0", "OC0A", "CKOUT", "PCINT10"])),
        io(6, "PA7", 7, s(&["ICP1", "OC0B", "ADC7", "PCINT7"])),
        io(7, "PA6", 6, s(&["MOSI", "DI", "SDA", "OC1A", "ADC6", "PCINT6"])),
        io(8, "PA5", 5, s(&["MISO", "DO", "OC1B", "ADC5", "PCINT5"])),
        io(9, "PA4", 4, s(&["SCK", "USCK", "SCL", "T1", "ADC4", "PCINT4"])),
        io(10, "PA3", 3, s(&["T0", "ADC3", "PCINT3"])),
        io(11, "PA2", 2, s(&["AIN1", "ADC2", "PCINT2"])),
        io(12, "PA1", 1, s(&["AIN0", "ADC1", "PCINT1"])),
        io(13, "PA0", 0, s(&["AREF", "ADC0", "PCINT0"])),
        power(14, "GND", PinKind::Gnd),
    ];
    let groups = [
        ("CPU", "CPU, Clock, Sleep, Reset & Power"),
        ("PORTA", "I/O Port A"),
        ("PORTB", "I/O Port B"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "8-bit Timer/Counter0 with PWM"),
        ("TC1", "16-bit Timer/Counter1 with PWM and input capture"),
        ("USI", "Universal Serial Interface"),
        ("AC", "Analog Comparator"),
        ("ADC", "10-bit Analog to Digital Converter"),
        ("EEPROM", "EEPROM"),
        ("WDT", "Watchdog Timer"),
    ];
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "tinyAVR (ATtiny24A/44A/84A)".into(),
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
        calibration: 0x80, // nominal: the real value is per chip
        fuses: fuses(),
        // MCUCR: SM1:0 = 00 idle, 01 ADC NR, 10 power-down, 11 standby.
        sleep: SleepControl {
            register: "MCUCR".into(),
            se_mask: 0x20,
            sm_mask: 0x18,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::Standby)],
        },
        boot: None,
        vectors: VECTOR_NAMES.iter().enumerate().map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() }).collect(),
        registers: registers(v),
        groups: groups.iter().map(|(n, d)| PeripheralGroupSpec { name: (*n).into(), desc: (*d).into() }).collect(),
        package: "PDIP-14".into(),
        pins,
        gpio_count: 12,
        has_adc: true,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Speed grades: 4 MHz @ 1.8-5.5 V, 10 MHz @ 2.7-5.5 V, 20 MHz @ 4.5-5.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (10e6, 2.7), (20e6, 4.5)],
        datasheet: "ATtiny24A/44A/84A datasheet Atmel-8183F (06/2012)".into(),
        die: None,
        peripheral_set: PeripheralSet::TinyX4,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
