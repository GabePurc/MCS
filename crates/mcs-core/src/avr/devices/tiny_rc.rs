//! ATtiny4 / ATtiny5 / ATtiny9 / ATtiny10 (AVRrc reduced core, SOT-23-6).
//! Source: Atmel-8127H ATtiny4/5/9/10 datasheet (11/2016) and avr-libc iotn10.h.

use crate::avr::device::*;
use crate::avr::isa::feature;

struct Variant {
    id: &'static str,
    name: &'static str,
    flash_size: u32,
    has_adc: bool,
    signature: [u8; 3],
}

const VARIANTS: &[Variant] = &[
    Variant { id: "attiny4", name: "ATtiny4", flash_size: 512, has_adc: false, signature: [0x1e, 0x8f, 0x0a] },
    Variant { id: "attiny5", name: "ATtiny5", flash_size: 512, has_adc: true, signature: [0x1e, 0x8f, 0x09] },
    Variant { id: "attiny9", name: "ATtiny9", flash_size: 1024, has_adc: false, signature: [0x1e, 0x90, 0x08] },
    Variant { id: "attiny10", name: "ATtiny10", flash_size: 1024, has_adc: true, signature: [0x1e, 0x90, 0x03] },
];

fn reg(name: &str, addr: u16, reset: u8, group: &str, desc: &str, bits: Vec<BitFieldSpec>) -> IoRegisterSpec {
    IoRegisterSpec { name: name.into(), addr, reset, group: group.into(), desc: desc.into(), bits, access: RegisterAccess::Rw }
}

fn port_bits(prefix: &str) -> Vec<BitFieldSpec> {
    let names: Vec<String> = (0..4).rev().map(|i| format!("{prefix}{i}")).collect();
    let refs: Vec<Option<&str>> = [None, None, None, None].into_iter().chain(names.iter().map(|s| Some(s.as_str()))).collect();
    bits_msb_first(&refs, &[])
}

fn registers(has_adc: bool) -> Vec<IoRegisterSpec> {
    let mut r = vec![
        reg("PINB", 0x00, 0, "PORTB", "Port B Input Pins (write 1 toggles PORTB bit)", port_bits("PINB")),
        reg("DDRB", 0x01, 0, "PORTB", "Port B Data Direction Register", port_bits("DDB")),
        reg("PORTB", 0x02, 0, "PORTB", "Port B Data Register", port_bits("PORTB")),
        reg("PUEB", 0x03, 0, "PORTB", "Port B Pull-up Enable Control Register", port_bits("PUEB")),
        reg("PORTCR", 0x0c, 0, "PORTB", "Port Control Register", vec![field("BBMB", 0x02, "Break-Before-Make Mode Enable")]),
        reg("PCMSK", 0x10, 0, "EXINT", "Pin Change Mask Register", port_bits("PCINT")),
        reg("PCIFR", 0x11, 0, "EXINT", "Pin Change Interrupt Flag Register", vec![field("PCIF0", 0x01, "Pin Change Interrupt Flag 0")]),
        reg("PCICR", 0x12, 0, "EXINT", "Pin Change Interrupt Control Register", vec![field("PCIE0", 0x01, "Pin Change Interrupt Enable 0")]),
        reg("EIMSK", 0x13, 0, "EXINT", "External Interrupt Mask Register", vec![field("INT0", 0x01, "External Interrupt Request 0 Enable")]),
        reg("EIFR", 0x14, 0, "EXINT", "External Interrupt Flag Register", vec![field("INTF0", 0x01, "External Interrupt Flag 0")]),
        reg("EICRA", 0x15, 0, "EXINT", "External Interrupt Control Register A", vec![field("ISC0", 0x03, "Interrupt Sense Control 0 (00 low, 01 any, 10 falling, 11 rising)")]),
        reg("DIDR0", 0x17, 0, "AC", "Digital Input Disable Register 0", port_bits("ADC").into_iter().map(|mut b| { b.name.push('D'); b }).collect()),
    ];
    if has_adc {
        let mut adcl = reg("ADCL", 0x19, 0, "ADC", "ADC Data Register (8-bit result)", vec![]);
        adcl.access = RegisterAccess::R;
        r.push(adcl);
        r.push(reg("ADMUX", 0x1b, 0, "ADC", "ADC Multiplexer Selection Register", vec![field("MUX", 0x03, "Analog Channel Selection (ADC0..ADC3)")]));
        r.push(reg("ADCSRB", 0x1c, 0, "ADC", "ADC Control and Status Register B", vec![field("ADTS", 0x07, "ADC Auto Trigger Source")]));
        r.push(reg("ADCSRA", 0x1d, 0, "ADC", "ADC Control and Status Register A", vec![
            field("ADEN", 0x80, "ADC Enable"), field("ADSC", 0x40, "ADC Start Conversion"),
            field("ADATE", 0x20, "ADC Auto Trigger Enable"), field("ADIF", 0x10, "ADC Interrupt Flag"),
            field("ADIE", 0x08, "ADC Interrupt Enable"), field("ADPS", 0x07, "ADC Prescaler Select"),
        ]));
    }
    r.extend([
        reg("ACSR", 0x1f, 0, "AC", "Analog Comparator Control and Status Register", vec![
            field("ACD", 0x80, "Analog Comparator Disable"), field("ACO", 0x20, "Analog Comparator Output"),
            field("ACI", 0x10, "Analog Comparator Interrupt Flag"), field("ACIE", 0x08, "Analog Comparator Interrupt Enable"),
            field("ACIC", 0x04, "Analog Comparator Input Capture Enable"), field("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("ICR0L", 0x22, 0, "TC0", "Input Capture Register Low Byte", vec![]),
        reg("ICR0H", 0x23, 0, "TC0", "Input Capture Register High Byte", vec![]),
        reg("OCR0BL", 0x24, 0, "TC0", "Output Compare Register B Low Byte", vec![]),
        reg("OCR0BH", 0x25, 0, "TC0", "Output Compare Register B High Byte", vec![]),
        reg("OCR0AL", 0x26, 0, "TC0", "Output Compare Register A Low Byte", vec![]),
        reg("OCR0AH", 0x27, 0, "TC0", "Output Compare Register A High Byte", vec![]),
        reg("TCNT0L", 0x28, 0, "TC0", "Timer/Counter0 Low Byte", vec![]),
        reg("TCNT0H", 0x29, 0, "TC0", "Timer/Counter0 High Byte", vec![]),
        reg("TIFR0", 0x2a, 0, "TC0", "Timer/Counter0 Interrupt Flag Register", vec![
            field("ICF0", 0x20, "Input Capture Flag"), field("OCF0B", 0x04, "Output Compare B Match Flag"),
            field("OCF0A", 0x02, "Output Compare A Match Flag"), field("TOV0", 0x01, "Overflow Flag"),
        ]),
        reg("TIMSK0", 0x2b, 0, "TC0", "Timer/Counter0 Interrupt Mask Register", vec![
            field("ICIE0", 0x20, "Input Capture Interrupt Enable"), field("OCIE0B", 0x04, "Output Compare B Match Interrupt Enable"),
            field("OCIE0A", 0x02, "Output Compare A Match Interrupt Enable"), field("TOIE0", 0x01, "Overflow Interrupt Enable"),
        ]),
        reg("TCCR0C", 0x2c, 0, "TC0", "Timer/Counter0 Control Register C", vec![field("FOC0A", 0x80, "Force Output Compare A"), field("FOC0B", 0x40, "Force Output Compare B")]),
        reg("TCCR0B", 0x2d, 0, "TC0", "Timer/Counter0 Control Register B", vec![
            field("ICNC0", 0x80, "Input Capture Noise Canceler"), field("ICES0", 0x40, "Input Capture Edge Select (1 = rising)"),
            field("WGM03", 0x10, "Waveform Generation Mode bit 3"), field("WGM02", 0x08, "Waveform Generation Mode bit 2"),
            field("CS0", 0x07, "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 T0 falling, 7 T0 rising)"),
        ]),
        reg("TCCR0A", 0x2e, 0, "TC0", "Timer/Counter0 Control Register A", vec![
            field("COM0A", 0xc0, "Compare Output Mode for Channel A"), field("COM0B", 0x30, "Compare Output Mode for Channel B"),
            field("WGM01", 0x02, "Waveform Generation Mode bit 1"), field("WGM00", 0x01, "Waveform Generation Mode bit 0"),
        ]),
        reg("GTCCR", 0x2f, 0, "TC0", "General Timer/Counter Control Register", vec![field("TSM", 0x80, "Timer/Counter Synchronization Mode"), field("PSR", 0x01, "Prescaler Reset")]),
        reg("WDTCSR", 0x31, 0, "WDT", "Watchdog Timer Control and Status Register", vec![
            field("WDIF", 0x80, "Watchdog Timer Interrupt Flag"), field("WDIE", 0x40, "Watchdog Timer Interrupt Enable"),
            field("WDP3", 0x20, "Watchdog Timer Prescaler bit 3"), field("WDE", 0x08, "Watchdog System Reset Enable"),
            field("WDP", 0x07, "Watchdog Timer Prescaler bits 2:0"),
        ]),
        reg("NVMCSR", 0x32, 0, "NVM", "Non-Volatile Memory Control and Status Register", vec![field("NVMBSY", 0x80, "NVM Busy")]),
        reg("NVMCMD", 0x33, 0, "NVM", "Non-Volatile Memory Command Register", vec![field("NVMCMD", 0x3f, "NVM Command")]),
        reg("VLMCSR", 0x34, 0, "VLM", "VCC Level Monitoring Control and Status Register", vec![
            field("VLMF", 0x80, "VLM Flag"), field("VLMIE", 0x40, "VLM Interrupt Enable"),
            field("VLM", 0x07, "Trigger Level (0 off, 1 VLM1L, 2 VLM1H, 3 VLM2, 4 VLM3)"),
        ]),
        reg("PRR", 0x35, 0, "CPU", "Power Reduction Register", vec![field("PRADC", 0x02, "Power Reduction ADC"), field("PRTIM0", 0x01, "Power Reduction Timer/Counter0")]),
        reg("CLKPSR", 0x36, 0x03, "CPU", "Clock Prescale Register (CCP protected)", vec![field("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("CLKMSR", 0x37, 0, "CPU", "Clock Main Settings Register (CCP protected)", vec![field("CLKMS", 0x03, "Clock Main Select (00 8MHz RC, 01 128kHz, 10 external)")]),
        reg("OSCCAL", 0x39, 0, "CPU", "Oscillator Calibration Register", vec![]),
        reg("SMCR", 0x3a, 0, "CPU", "Sleep Mode Control Register", vec![field("SM", 0x0e, "Sleep Mode (000 idle, 001 ADC NR, 010 power-down, 100 standby)"), field("SE", 0x01, "Sleep Enable")]),
        reg("RSTFLR", 0x3b, 0, "CPU", "Reset Flag Register", vec![field("WDRF", 0x08, "Watchdog Reset Flag"), field("EXTRF", 0x02, "External Reset Flag"), field("PORF", 0x01, "Power-on Reset Flag")]),
        {
            let mut ccp = reg("CCP", 0x3c, 0, "CPU", "Configuration Change Protection (write 0xD8 to unlock for 4 cycles)", vec![]);
            ccp.access = RegisterAccess::W;
            ccp
        },
        reg("SPL", 0x3d, 0x5f, "CPU", "Stack Pointer Low Byte", vec![]),
        reg("SPH", 0x3e, 0x00, "CPU", "Stack Pointer High Byte", vec![]),
        reg("SREG", 0x3f, 0, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
    ]);
    r
}

fn vectors(has_adc: bool) -> Vec<VectorSpec> {
    let mut v: Vec<VectorSpec> = [
        ("RESET", "External Pin, Power-on Reset, VLM Reset, Watchdog Reset"),
        ("INT0", "External Interrupt Request 0"),
        ("PCINT0", "Pin Change Interrupt Request 0"),
        ("TIM0_CAPT", "Timer/Counter0 Input Capture"),
        ("TIM0_OVF", "Timer/Counter0 Overflow"),
        ("TIM0_COMPA", "Timer/Counter0 Compare Match A"),
        ("TIM0_COMPB", "Timer/Counter0 Compare Match B"),
        ("ANA_COMP", "Analog Comparator"),
        ("WDT", "Watchdog Time-out"),
        ("VLM", "VCC Voltage Level Monitor"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (n, d))| VectorSpec { index: i as u8, name: (*n).into(), desc: (*d).into() })
    .collect();
    if has_adc {
        v.push(VectorSpec { index: 10, name: "ADC".into(), desc: "ADC Conversion Complete".into() });
    }
    v
}

fn spec(v: &Variant) -> AvrDeviceSpec {
    let adc = |n: u8| if v.has_adc { vec![format!("ADC{n}")] } else { vec![] };
    let pin = |number: u8, name: &str, gpio: Option<u8>, kind: PinKind, functions: Vec<String>| PinSpec { number, name: name.into(), kind, gpio, functions };
    let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut groups = vec![
        ("CPU", "CPU, Clock, Sleep & Reset"),
        ("PORTB", "I/O Port B"),
        ("EXINT", "External & Pin Change Interrupts"),
        ("TC0", "16-bit Timer/Counter0"),
        ("AC", "Analog Comparator"),
    ];
    if v.has_adc {
        groups.push(("ADC", "8-bit Analog to Digital Converter"));
    }
    groups.extend([("WDT", "Watchdog Timer"), ("VLM", "VCC Level Monitor"), ("NVM", "Non-Volatile Memory Controller")]);
    AvrDeviceSpec {
        id: v.id.into(),
        name: v.name.into(),
        family: "tinyAVR (ATtiny4/5/9/10)".into(),
        core_name: "AVRrc".into(),
        features: feature::RC | feature::BREAK,
        flash_size: v.flash_size,
        sram_start: 0x40,
        sram_size: 32,
        eeprom_size: 0,
        io_base: 0,
        io_size: 64,
        regs_in_data_space: false,
        flash_map_base: Some(0x4000),
        nvm_map: Some(NvmMap { lock: 0x3f00, config: 0x3f40, calibration: 0x3f80, signature: 0x3fc0 }),
        signature: v.signature,
        calibration: 0x9c,
        fuses: vec![FuseByteSpec {
            name: "Configuration".into(),
            default: 0xff,
            bits: vec![
                FuseBitSpec { name: "RSTDISBL".into(), mask: 0x01, desc: "External Reset disabled (PB3 becomes I/O) when programmed (0)".into() },
                FuseBitSpec { name: "WDTON".into(), mask: 0x02, desc: "Watchdog Timer always on when programmed (0)".into() },
                FuseBitSpec { name: "CKOUT".into(), mask: 0x04, desc: "System clock output on PB2 when programmed (0)".into() },
            ],
        }],
        // Section 7.1 / SMCR: SM2:0 = 000 idle, 001 ADC NR, 010 power-down, 100 standby.
        sleep: SleepControl {
            register: "SMCR".into(),
            se_mask: 0x01,
            sm_mask: 0x0e,
            modes: vec![(0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (4, SleepKind::Standby)],
        },
        boot: None,
        vectors: vectors(v.has_adc),
        registers: registers(v.has_adc),
        groups: groups.into_iter().map(|(n, d)| PeripheralGroupSpec { name: n.into(), desc: d.into() }).collect(),
        package: "SOT-23-6".into(),
        pins: vec![
            pin(1, "PB0", Some(0), PinKind::Io, [s(&["TPIDATA", "OC0A"]), adc(0), s(&["AIN0", "PCINT0"])].concat()),
            pin(2, "GND", None, PinKind::Gnd, vec![]),
            pin(3, "PB1", Some(1), PinKind::Io, [s(&["TPICLK", "CLKI", "ICP0", "OC0B"]), adc(1), s(&["AIN1", "PCINT1"])].concat()),
            pin(4, "PB2", Some(2), PinKind::Io, [s(&["T0", "CLKO"]), adc(2), s(&["INT0", "PCINT2"])].concat()),
            pin(5, "VCC", None, PinKind::Vcc, vec![]),
            pin(6, "PB3", Some(3), PinKind::Io, [s(&["RESET"]), adc(3), s(&["PCINT3"])].concat()),
        ],
        gpio_count: 4,
        has_adc: v.has_adc,
        clock: ClockSpec { internal_hz: 8_000_000.0, slow_hz: 128_000.0, default_prescale_log2: 3 },
        vcc: 5.0,
        // Section 16.3 "Speed": 0-4 MHz @ 1.8-5.5 V, 0-8 MHz @ 2.7-5.5 V, 0-12 MHz @ 4.5-5.5 V.
        vcc_range: (1.8, 5.5),
        speed_grades: vec![(4e6, 1.8), (8e6, 2.7), (12e6, 4.5)],
        datasheet: "Atmel-8127H ATtiny4/5/9/10 datasheet (11/2016)".into(),
        // Die size measured by Zeptobars on a decapped ATtiny4 (zeptobars.com, 2019-02-01).
        die: Some(DieSpec {
            width_um: 1368.0,
            height_um: 926.0,
            photo_url: "https://zeptobars.com/en/read/atmel-tiny4-attiny4-microcontroller".into(),
            photo_credit: "ATtiny4 die photo by Zeptobars, CC BY 3.0".into(),
        }),
        peripheral_set: PeripheralSet::TinyRc,
    }
}

pub fn devices() -> Vec<AvrDeviceSpec> {
    VARIANTS.iter().map(spec).collect()
}
