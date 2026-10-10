//! Register and vector builders shared by the two large ATmega families, [`super::mega_x4`]
//! (ATmega164PA/324PA/644PA/1284P) and [`super::mega_x0`] (ATmega640/1280/2560). Both follow the
//! ATmega328P register generation (TCCRnA/B, TIFRn/TIMSKn, UCSRnA/B/C, PRRn, ...) and differ in
//! the instance counts, addresses and pinout, which the callers supply.
//!
//! Sources: Atmel-8272G ATmega164A/PA/324A/PA/644A/PA/1284/P (01/2015) and Atmel-2549Q
//! ATmega640/1280/1281/2560/2561 (02/2014) register summaries; avr-libc iomxx4.h / iom1284p.h and
//! iomxx0_1.h for addresses and bit names.

use crate::avr::device::*;

pub(super) fn reg(name: &str, addr: u16, group: &str, desc: &str, bits: Vec<BitFieldSpec>) -> IoRegisterSpec {
    IoRegisterSpec { name: name.into(), addr, reset: 0, group: group.into(), desc: desc.into(), bits, access: RegisterAccess::Rw }
}

pub(super) fn reset(mut r: IoRegisterSpec, v: u8) -> IoRegisterSpec {
    r.reset = v;
    r
}

pub(super) fn ro(mut r: IoRegisterSpec) -> IoRegisterSpec {
    r.access = RegisterAccess::R;
    r
}

/// Single-bit fields named `prefix7..prefix0` for the bits set in `mask`.
pub(super) fn nbits(prefix: &str, mask: u8) -> Vec<BitFieldSpec> {
    (0..8).rev().filter(|i| mask & (1 << i) != 0).map(|i| field(&format!("{prefix}{i}"), 1 << i, "")).collect()
}

pub(super) fn b(name: &str, mask: u8, desc: &str) -> BitFieldSpec {
    field(name, mask, desc)
}

/// Port registers PINx / DDRx / PORTx: `(letter, PINx address, implemented pins mask)`.
pub(super) fn ports(list: &[(char, u16, u8)]) -> Vec<IoRegisterSpec> {
    let mut r = Vec::new();
    for &(l, base, mask) in list {
        let grp = format!("PORT{l}");
        r.push(reg(&format!("PIN{l}"), base, &grp, &format!("Port {l} Input Pins (write 1 toggles PORT{l} bit)"), nbits(&format!("PIN{l}"), mask)));
        r.push(reg(&format!("DDR{l}"), base + 1, &grp, &format!("Port {l} Data Direction Register"), nbits(&format!("DD{l}"), mask)));
        r.push(reg(&format!("PORT{l}"), base + 2, &grp, &format!("Port {l} Data Register"), nbits(&format!("PORT{l}"), mask)));
    }
    r
}

const CS_STD: &str = "Clock Select (0 stop, 1 /1, 2 /8, 3 /64, 4 /256, 5 /1024, 6 Tn falling, 7 Tn rising)";
const CS_T2: &str = "Clock Select (0 stop, 1 /1, 2 /8, 3 /32, 4 /64, 5 /128, 6 /256, 7 /1024)";

/// 8-bit Timer/Counter n (TC0, TC2): control registers at `base`, flags in `tifr` / `timsk`.
pub(super) fn timer8(n: u8, base: u16, tifr: u16, timsk: u16) -> Vec<IoRegisterSpec> {
    let grp = format!("TC{n}");
    vec![
        reg(&format!("TCCR{n}A"), base, &grp, &format!("Timer/Counter{n} Control Register A"), vec![
            b(&format!("COM{n}A"), 0xc0, "Compare Output Mode A"), b(&format!("COM{n}B"), 0x30, "Compare Output Mode B"),
            b(&format!("WGM{n}1"), 0x02, "Waveform Generation Mode bit 1"), b(&format!("WGM{n}0"), 0x01, "Waveform Generation Mode bit 0"),
        ]),
        reg(&format!("TCCR{n}B"), base + 1, &grp, &format!("Timer/Counter{n} Control Register B"), vec![
            b(&format!("FOC{n}A"), 0x80, "Force Output Compare A"), b(&format!("FOC{n}B"), 0x40, "Force Output Compare B"),
            b(&format!("WGM{n}2"), 0x08, "Waveform Generation Mode bit 2"), b(&format!("CS{n}"), 0x07, if n == 2 { CS_T2 } else { CS_STD }),
        ]),
        reg(&format!("TCNT{n}"), base + 2, &grp, &format!("Timer/Counter{n}"), vec![]),
        reg(&format!("OCR{n}A"), base + 3, &grp, &format!("Output Compare Register {n} A"), vec![]),
        reg(&format!("OCR{n}B"), base + 4, &grp, &format!("Output Compare Register {n} B"), vec![]),
        reg(&format!("TIFR{n}"), tifr, &grp, &format!("Timer/Counter{n} Interrupt Flag Register"), vec![
            b(&format!("OCF{n}B"), 0x04, "Output Compare B Match Flag"), b(&format!("OCF{n}A"), 0x02, "Output Compare A Match Flag"), b(&format!("TOV{n}"), 0x01, "Overflow Flag"),
        ]),
        reg(&format!("TIMSK{n}"), timsk, &grp, &format!("Timer/Counter{n} Interrupt Mask Register"), vec![
            b(&format!("OCIE{n}B"), 0x04, "Output Compare B Match Interrupt Enable"), b(&format!("OCIE{n}A"), 0x02, "Output Compare A Match Interrupt Enable"), b(&format!("TOIE{n}"), 0x01, "Overflow Interrupt Enable"),
        ]),
    ]
}

/// Timer2 asynchronous status register (asynchronous operation itself is not simulated).
pub(super) fn assr() -> IoRegisterSpec {
    reg("ASSR", 0xb6, "TC2", "Asynchronous Status Register", vec![
        b("EXCLK", 0x40, "Enable External Clock Input"), b("AS2", 0x20, "Asynchronous Timer/Counter2"), b("TCN2UB", 0x10, "TCNT2 Update Busy"),
        b("OCR2AUB", 0x08, "OCR2A Update Busy"), b("OCR2BUB", 0x04, "OCR2B Update Busy"), b("TCR2AUB", 0x02, "TCCR2A Update Busy"), b("TCR2BUB", 0x01, "TCCR2B Update Busy"),
    ])
}

/// 16-bit Timer/Counter n with two compare units, or three (`with_c`, ATmega640/1280/2560).
pub(super) fn timer16(n: u8, base: u16, tifr: u16, timsk: u16, with_c: bool) -> Vec<IoRegisterSpec> {
    let grp = format!("TC{n}");
    let mut a = vec![
        b(&format!("COM{n}A"), 0xc0, "Compare Output Mode A"), b(&format!("COM{n}B"), 0x30, "Compare Output Mode B"),
    ];
    if with_c {
        a.push(b(&format!("COM{n}C"), 0x0c, "Compare Output Mode C"));
    }
    a.push(b(&format!("WGM{n}1"), 0x02, "Waveform Generation Mode bit 1"));
    a.push(b(&format!("WGM{n}0"), 0x01, "Waveform Generation Mode bit 0"));
    let mut foc = vec![b(&format!("FOC{n}A"), 0x80, "Force Output Compare A"), b(&format!("FOC{n}B"), 0x40, "Force Output Compare B")];
    let mut flags = vec![b(&format!("ICF{n}"), 0x20, "Input Capture Flag")];
    let mut mask = vec![b(&format!("ICIE{n}"), 0x20, "Input Capture Interrupt Enable")];
    if with_c {
        foc.push(b(&format!("FOC{n}C"), 0x20, "Force Output Compare C"));
        flags.push(b(&format!("OCF{n}C"), 0x08, "Output Compare C Match Flag"));
        mask.push(b(&format!("OCIE{n}C"), 0x08, "Output Compare C Match Interrupt Enable"));
    }
    flags.extend([b(&format!("OCF{n}B"), 0x04, "Output Compare B Match Flag"), b(&format!("OCF{n}A"), 0x02, "Output Compare A Match Flag"), b(&format!("TOV{n}"), 0x01, "Overflow Flag")]);
    mask.extend([
        b(&format!("OCIE{n}B"), 0x04, "Output Compare B Match Interrupt Enable"), b(&format!("OCIE{n}A"), 0x02, "Output Compare A Match Interrupt Enable"),
        b(&format!("TOIE{n}"), 0x01, "Overflow Interrupt Enable"),
    ]);
    let mut r = vec![
        reg(&format!("TCCR{n}A"), base, &grp, &format!("Timer/Counter{n} Control Register A"), a),
        reg(&format!("TCCR{n}B"), base + 1, &grp, &format!("Timer/Counter{n} Control Register B"), vec![
            b(&format!("ICNC{n}"), 0x80, "Input Capture Noise Canceler"), b(&format!("ICES{n}"), 0x40, "Input Capture Edge Select (1 = rising)"),
            b(&format!("WGM{n}3"), 0x10, "Waveform Generation Mode bit 3"), b(&format!("WGM{n}2"), 0x08, "Waveform Generation Mode bit 2"), b(&format!("CS{n}"), 0x07, CS_STD),
        ]),
        reg(&format!("TCCR{n}C"), base + 2, &grp, &format!("Timer/Counter{n} Control Register C"), foc),
        reg(&format!("TCNT{n}L"), base + 4, &grp, &format!("Timer/Counter{n} Low Byte"), vec![]),
        reg(&format!("TCNT{n}H"), base + 5, &grp, &format!("Timer/Counter{n} High Byte"), vec![]),
        reg(&format!("ICR{n}L"), base + 6, &grp, &format!("Input Capture Register {n} Low Byte"), vec![]),
        reg(&format!("ICR{n}H"), base + 7, &grp, &format!("Input Capture Register {n} High Byte"), vec![]),
    ];
    for (i, ch) in ["A", "B", "C"].into_iter().take(if with_c { 3 } else { 2 }).enumerate() {
        r.push(reg(&format!("OCR{n}{ch}L"), base + 8 + 2 * i as u16, &grp, &format!("Output Compare Register {n} {ch} Low Byte"), vec![]));
        r.push(reg(&format!("OCR{n}{ch}H"), base + 9 + 2 * i as u16, &grp, &format!("Output Compare Register {n} {ch} High Byte"), vec![]));
    }
    r.push(reg(&format!("TIFR{n}"), tifr, &grp, &format!("Timer/Counter{n} Interrupt Flag Register"), flags));
    r.push(reg(&format!("TIMSK{n}"), timsk, &grp, &format!("Timer/Counter{n} Interrupt Mask Register"), mask));
    r
}

pub(super) fn usart(k: u8, base: u16) -> Vec<IoRegisterSpec> {
    let grp = format!("USART{k}");
    vec![
        reset(reg(&format!("UCSR{k}A"), base, &grp, "USART Control and Status Register A", vec![
            b(&format!("RXC{k}"), 0x80, "Receive Complete"), b(&format!("TXC{k}"), 0x40, "Transmit Complete"), b(&format!("UDRE{k}"), 0x20, "Data Register Empty"), b(&format!("FE{k}"), 0x10, "Frame Error"),
            b(&format!("DOR{k}"), 0x08, "Data OverRun"), b(&format!("UPE{k}"), 0x04, "Parity Error"), b(&format!("U2X{k}"), 0x02, "Double Transmission Speed"), b(&format!("MPCM{k}"), 0x01, "Multi-processor Communication Mode"),
        ]), 0x20),
        reg(&format!("UCSR{k}B"), base + 1, &grp, "USART Control and Status Register B", vec![
            b(&format!("RXCIE{k}"), 0x80, "RX Complete Interrupt Enable"), b(&format!("TXCIE{k}"), 0x40, "TX Complete Interrupt Enable"), b(&format!("UDRIE{k}"), 0x20, "Data Register Empty Interrupt Enable"),
            b(&format!("RXEN{k}"), 0x10, "Receiver Enable"), b(&format!("TXEN{k}"), 0x08, "Transmitter Enable"), b(&format!("UCSZ{k}2"), 0x04, "Character Size bit 2"), b(&format!("RXB8{k}"), 0x02, "Receive Data Bit 8"), b(&format!("TXB8{k}"), 0x01, "Transmit Data Bit 8"),
        ]),
        reset(reg(&format!("UCSR{k}C"), base + 2, &grp, "USART Control and Status Register C", vec![
            b(&format!("UMSEL{k}"), 0xc0, "USART Mode Select (00 asynchronous)"), b(&format!("UPM{k}"), 0x30, "Parity Mode (00 none, 10 even, 11 odd)"), b(&format!("USBS{k}"), 0x08, "Stop Bit Select (1 = 2 stop bits)"),
            b(&format!("UCSZ{k}"), 0x06, "Character Size bits 1:0 (11 = 8 bits)"), b(&format!("UCPOL{k}"), 0x01, "Clock Polarity"),
        ]), 0x06),
        reg(&format!("UBRR{k}L"), base + 4, &grp, "USART Baud Rate Register Low Byte", vec![]),
        reg(&format!("UBRR{k}H"), base + 5, &grp, "USART Baud Rate Register High Byte", vec![]),
        reg(&format!("UDR{k}"), base + 6, &grp, "USART I/O Data Register", vec![]),
    ]
}

pub(super) fn spi() -> Vec<IoRegisterSpec> {
    vec![
        reg("SPCR", 0x4c, "SPI", "SPI Control Register", vec![
            b("SPIE", 0x80, "SPI Interrupt Enable"), b("SPE", 0x40, "SPI Enable"), b("DORD", 0x20, "Data Order (1 = LSB first)"), b("MSTR", 0x10, "Master/Slave Select"),
            b("CPOL", 0x08, "Clock Polarity"), b("CPHA", 0x04, "Clock Phase"), b("SPR", 0x03, "SPI Clock Rate Select (fosc/4, /16, /64, /128)"),
        ]),
        reg("SPSR", 0x4d, "SPI", "SPI Status Register", vec![b("SPIF", 0x80, "SPI Interrupt Flag"), b("WCOL", 0x40, "Write Collision Flag"), b("SPI2X", 0x01, "Double SPI Speed")]),
        reg("SPDR", 0x4e, "SPI", "SPI Data Register", vec![]),
    ]
}

pub(super) fn twi() -> Vec<IoRegisterSpec> {
    vec![
        reg("TWBR", 0xb8, "TWI", "TWI Bit Rate Register", vec![]),
        reset(reg("TWSR", 0xb9, "TWI", "TWI Status Register", vec![b("TWS", 0xf8, "TWI Status"), b("TWPS", 0x03, "TWI Prescaler (1, 4, 16, 64)")]), 0xf8),
        reset(reg("TWAR", 0xba, "TWI", "TWI (Slave) Address Register", vec![b("TWA", 0xfe, "TWI Slave Address"), b("TWGCE", 0x01, "TWI General Call Recognition Enable")]), 0xfe),
        reset(reg("TWDR", 0xbb, "TWI", "TWI Data Register", vec![]), 0xff),
        reg("TWCR", 0xbc, "TWI", "TWI Control Register", vec![
            b("TWINT", 0x80, "TWI Interrupt Flag"), b("TWEA", 0x40, "TWI Enable Acknowledge"), b("TWSTA", 0x20, "TWI START Condition"), b("TWSTO", 0x10, "TWI STOP Condition"),
            b("TWWC", 0x08, "TWI Write Collision"), b("TWEN", 0x04, "TWI Enable"), b("TWIE", 0x01, "TWI Interrupt Enable"),
        ]),
        reg("TWAMR", 0xbd, "TWI", "TWI (Slave) Address Mask Register", vec![]),
    ]
}

pub(super) fn eeprom() -> Vec<IoRegisterSpec> {
    vec![
        reg("EECR", 0x3f, "EEPROM", "EEPROM Control Register", vec![
            b("EEPM", 0x30, "EEPROM Programming Mode (00 erase+write, 01 erase, 10 write)"), b("EERIE", 0x08, "EEPROM Ready Interrupt Enable"),
            b("EEMPE", 0x04, "EEPROM Master Write Enable"), b("EEPE", 0x02, "EEPROM Write Enable"), b("EERE", 0x01, "EEPROM Read Enable"),
        ]),
        reg("EEDR", 0x40, "EEPROM", "EEPROM Data Register", vec![]),
        reg("EEARL", 0x41, "EEPROM", "EEPROM Address Register Low Byte", vec![]),
        reg("EEARH", 0x42, "EEPROM", "EEPROM Address Register High Byte", vec![]),
    ]
}

pub(super) fn analog_comparator() -> Vec<IoRegisterSpec> {
    vec![
        reg("ACSR", 0x50, "AC", "Analog Comparator Control and Status Register", vec![
            b("ACD", 0x80, "Analog Comparator Disable"), b("ACBG", 0x40, "Bandgap Select (1.1 V on the positive input)"), b("ACO", 0x20, "Analog Comparator Output"),
            b("ACI", 0x10, "Analog Comparator Interrupt Flag"), b("ACIE", 0x08, "Analog Comparator Interrupt Enable"), b("ACIC", 0x04, "Input Capture Enable (Timer1)"),
            b("ACIS", 0x03, "Interrupt Mode Select (00 toggle, 10 falling, 11 rising)"),
        ]),
        reg("DIDR1", 0x7f, "AC", "Digital Input Disable Register 1", vec![b("AIN1D", 0x02, "AIN1 Digital Input Disable"), b("AIN0D", 0x01, "AIN0 Digital Input Disable")]),
    ]
}

/// ADC registers; `mux5` adds ADCSRB.MUX5 (ATmega640/1280/2560).
pub(super) fn adc(mux5: bool) -> Vec<IoRegisterSpec> {
    let mut srb = vec![b("ACME", 0x40, "Analog Comparator Multiplexer Enable")];
    if mux5 {
        srb.push(b("MUX5", 0x08, "Analog Channel and Gain Selection bit 5 (channels 8-15 and their differential pairs)"));
    }
    srb.push(b("ADTS", 0x07, "ADC Auto Trigger Source"));
    vec![
        ro(reg("ADCL", 0x78, "ADC", "ADC Data Register Low Byte (read first)", vec![])),
        ro(reg("ADCH", 0x79, "ADC", "ADC Data Register High Byte", vec![])),
        reg("ADCSRA", 0x7a, "ADC", "ADC Control and Status Register A", vec![
            b("ADEN", 0x80, "ADC Enable"), b("ADSC", 0x40, "ADC Start Conversion"), b("ADATE", 0x20, "ADC Auto Trigger Enable"),
            b("ADIF", 0x10, "ADC Interrupt Flag"), b("ADIE", 0x08, "ADC Interrupt Enable"), b("ADPS", 0x07, "ADC Prescaler Select"),
        ]),
        reg("ADCSRB", 0x7b, "ADC", "ADC Control and Status Register B", srb),
        reg("ADMUX", 0x7c, "ADC", "ADC Multiplexer Selection Register", vec![
            b("REFS", 0xc0, "Reference Selection (00 AREF, 01 AVCC, 10 internal 1.1 V, 11 internal 2.56 V)"), b("ADLAR", 0x20, "ADC Left Adjust Result"),
            b("MUX", 0x1f, "Analog Channel and Gain Selection (0-7 ADCn, 8-15 differential 10x/200x, 16-29 differential 1x, 30 1.1 V, 31 GND)"),
        ]),
    ]
}

/// What differs between the two families in the CPU / system register block.
pub(super) struct Cpu {
    pub sram_end: u16,
    /// BODS / BODSE in MCUCR (picoPower parts).
    pub bod_sleep: bool,
    /// RAMPZ bits implemented (0 = no register).
    pub rampz_mask: u8,
    pub eind: bool,
    pub prr0: Vec<BitFieldSpec>,
    pub prr1: Vec<BitFieldSpec>,
}

pub(super) fn cpu(c: &Cpu) -> Vec<IoRegisterSpec> {
    let mut mcucr = vec![b("JTD", 0x80, "JTAG Interface Disable (JTAG is not modelled)")];
    if c.bod_sleep {
        mcucr.push(b("BODS", 0x40, "BOD Sleep"));
        mcucr.push(b("BODSE", 0x20, "BOD Sleep Enable"));
    }
    mcucr.extend([b("PUD", 0x10, "Pull-up Disable"), b("IVSEL", 0x02, "Interrupt Vector Select (boot section)"), b("IVCE", 0x01, "Interrupt Vector Change Enable")]);
    let mut r = vec![
        reg("GPIOR0", 0x3e, "CPU", "General Purpose I/O Register 0", vec![]),
        reg("GPIOR1", 0x4a, "CPU", "General Purpose I/O Register 1", vec![]),
        reg("GPIOR2", 0x4b, "CPU", "General Purpose I/O Register 2", vec![]),
        reg("OCDR", 0x51, "CPU", "On-chip Debug Register (not modelled)", vec![]),
        reg("SMCR", 0x53, "CPU", "Sleep Mode Control Register", vec![b("SM", 0x0e, "Sleep Mode (000 idle, 001 ADC NR, 010 power-down, 011 power-save, 110 standby, 111 ext. standby)"), b("SE", 0x01, "Sleep Enable")]),
        reg("MCUSR", 0x54, "CPU", "MCU Status Register (reset flags)", vec![
            b("JTRF", 0x10, "JTAG Reset Flag (JTAG is not modelled)"), b("WDRF", 0x08, "Watchdog Reset Flag"), b("BORF", 0x04, "Brown-out Reset Flag"),
            b("EXTRF", 0x02, "External Reset Flag"), b("PORF", 0x01, "Power-on Reset Flag"),
        ]),
        reg("MCUCR", 0x55, "CPU", "MCU Control Register", mcucr),
        reg("SPMCSR", 0x57, "CPU", "Store Program Memory Control and Status Register", vec![
            b("SPMIE", 0x80, "SPM Interrupt Enable"), b("RWWSB", 0x40, "Read-While-Write Section Busy"), b("SIGRD", 0x20, "Signature Row Read"), b("RWWSRE", 0x10, "RWW Section Read Enable"),
            b("BLBSET", 0x08, "Boot Lock Bit Set"), b("PGWRT", 0x04, "Page Write"), b("PGERS", 0x02, "Page Erase"), b("SPMEN", 0x01, "Store Program Memory Enable"),
        ]),
        reset(reg("SPL", 0x5d, "CPU", "Stack Pointer Low Byte", vec![]), (c.sram_end & 0xff) as u8),
        reset(reg("SPH", 0x5e, "CPU", "Stack Pointer High Byte", vec![]), (c.sram_end >> 8) as u8),
        reg("SREG", 0x5f, "CPU", "Status Register", bits_msb_first(
            &[Some("I"), Some("T"), Some("H"), Some("S"), Some("V"), Some("N"), Some("Z"), Some("C")],
            &[("I", "Global Interrupt Enable"), ("T", "Bit Copy Storage"), ("H", "Half Carry Flag"), ("S", "Sign Bit (N xor V)"),
              ("V", "Two's Complement Overflow Flag"), ("N", "Negative Flag"), ("Z", "Zero Flag"), ("C", "Carry Flag")],
        )),
        reg("WDTCSR", 0x60, "WDT", "Watchdog Timer Control Register", vec![
            b("WDIF", 0x80, "Watchdog Interrupt Flag"), b("WDIE", 0x40, "Watchdog Interrupt Enable"), b("WDP3", 0x20, "Watchdog Prescaler bit 3"),
            b("WDCE", 0x10, "Watchdog Change Enable"), b("WDE", 0x08, "Watchdog System Reset Enable"), b("WDP", 0x07, "Watchdog Prescaler bits 2:0"),
        ]),
        reg("CLKPR", 0x61, "CPU", "Clock Prescale Register", vec![b("CLKPCE", 0x80, "Clock Prescaler Change Enable"), b("CLKPS", 0x0f, "Clock Prescaler Select (division = 2^CLKPS)")]),
        reg("PRR0", 0x64, "CPU", "Power Reduction Register 0", c.prr0.clone()),
        reg("OSCCAL", 0x66, "CPU", "Oscillator Calibration Register", vec![]),
    ];
    if !c.prr1.is_empty() {
        r.push(reg("PRR1", 0x65, "CPU", "Power Reduction Register 1", c.prr1.clone()));
    }
    if c.rampz_mask != 0 {
        r.push(reg("RAMPZ", 0x5b, "CPU", "Extended Z-pointer Register (flash bits 23:16 for ELPM/SPM)", nbits("RAMPZ", c.rampz_mask)));
    }
    if c.eind {
        r.push(reg("EIND", 0x5c, "CPU", "Extended Indirect Register (bits 21:16 of EIJMP/EICALL)", nbits("EIND", 0x01)));
    }
    r
}

/// Fuse bytes of both families (identical layout): low = clock, high = JTAG/OCD, SPI, WDT, EEPROM
/// and boot loader, extended = BOD level.
pub(super) fn fuses(high_default: u8) -> Vec<FuseByteSpec> {
    let f = |n: &str, m: u8, d: &str| FuseBitSpec { name: n.into(), mask: m, desc: d.into() };
    vec![
        FuseByteSpec {
            name: "Low".into(),
            default: 0x62,
            bits: vec![
                f("CKDIV8", 0x80, "Divide clock by 8 at reset (CLKPR = /8) when programmed (0)"),
                f("CKOUT", 0x40, "Clock output (CLKO) when programmed (0)"),
                f("SUT", 0x30, "Start-up time select"),
                f("CKSEL", 0x0f, "Clock source (0000 external clock, 0010 internal 8 MHz RC, 0011 internal 128 kHz, 0100-0101 32 kHz crystal, 0110-0111 full-swing crystal, 1000-1111 low-power crystal)"),
            ],
        },
        FuseByteSpec {
            name: "High".into(),
            default: high_default,
            bits: vec![
                f("OCDEN", 0x80, "On-chip debug enabled when programmed (0; not modelled)"),
                f("JTAGEN", 0x40, "JTAG interface enabled when programmed (0; the JTAG pins stay ordinary GPIO here)"),
                f("SPIEN", 0x20, "Serial programming enabled when programmed (0)"),
                f("WDTON", 0x10, "Watchdog Timer always on when programmed (0)"),
                f("EESAVE", 0x08, "EEPROM preserved through chip erase when programmed (0)"),
                f("BOOTSZ", 0x06, "Boot section size (see boot loader table)"),
                f("BOOTRST", 0x01, "Reset to the boot loader section when programmed (0)"),
            ],
        },
        FuseByteSpec {
            name: "Extended".into(),
            default: 0xff,
            bits: vec![f("BODLEVEL", 0x07, "Brown-out detector level (111 disabled, 110 1.8 V, 101 2.7 V, 100 4.3 V)")],
        },
    ]
}

/// Sleep control of both families (SMCR, section "Sleep Mode Control Register").
pub(super) fn sleep() -> SleepControl {
    SleepControl {
        register: "SMCR".into(),
        se_mask: 0x01,
        sm_mask: 0x0e,
        modes: vec![
            (0, SleepKind::Idle), (1, SleepKind::AdcNoiseReduction), (2, SleepKind::PowerDown), (3, SleepKind::PowerSave),
            (6, SleepKind::Standby), (7, SleepKind::ExtendedStandby),
        ],
    }
}

fn describe_vector(name: &str) -> String {
    let num = |p: &str| name.strip_prefix(p).map(str::to_string);
    if name == "RESET" {
        return "External Pin, Power-on Reset, Brown-out Reset, Watchdog System Reset and JTAG AVR Reset".into();
    }
    if let Some(n) = num("INT") {
        return format!("External Interrupt Request {n}");
    }
    if let Some(n) = num("PCINT") {
        return format!("Pin Change Interrupt Request {n}");
    }
    if let Some(rest) = name.strip_prefix("TIMER") {
        if let Some((n, ev)) = rest.split_once('_') {
            let what = match ev {
                "CAPT" => "Capture Event".to_string(),
                "OVF" => "Overflow".to_string(),
                c => format!("Compare Match {}", c.trim_start_matches("COMP")),
            };
            return format!("Timer/Counter{n} {what}");
        }
    }
    if let Some(rest) = name.strip_prefix("USART") {
        if let Some((n, ev)) = rest.split_once('_') {
            let what = match ev {
                "RX" => "Rx Complete",
                "UDRE" => "Data Register Empty",
                _ => "Tx Complete",
            };
            return format!("USART{n} {what}");
        }
    }
    match name {
        "WDT" => "Watchdog Time-out Interrupt",
        "SPI_STC" => "SPI Serial Transfer Complete",
        "ANALOG_COMP" => "Analog Comparator",
        "ADC" => "ADC Conversion Complete",
        "EE_READY" => "EEPROM Ready",
        "TWI" => "2-wire Serial Interface",
        "SPM_READY" => "Store Program Memory Ready",
        _ => "",
    }
    .into()
}

/// Vector table in order (index 0 = RESET) with generated descriptions.
pub(super) fn vectors(names: &[String]) -> Vec<VectorSpec> {
    names.iter().enumerate().map(|(i, n)| VectorSpec { index: i as u8, name: n.clone(), desc: describe_vector(n) }).collect()
}

/// `TIMERn_CAPT, _COMPA, _COMPB[, _COMPC], _OVF` vector names.
pub(super) fn timer_vectors(n: u8, with_c: bool) -> Vec<String> {
    let mut v = vec![format!("TIMER{n}_CAPT"), format!("TIMER{n}_COMPA"), format!("TIMER{n}_COMPB")];
    if with_c {
        v.push(format!("TIMER{n}_COMPC"));
    }
    v.push(format!("TIMER{n}_OVF"));
    v
}

pub(super) fn usart_vectors(k: u8) -> Vec<String> {
    ["RX", "UDRE", "TX"].iter().map(|e| format!("USART{k}_{e}")).collect()
}

/// Pin helpers for the pinout tables.
pub(super) fn io(number: u8, name: &str, gpio: u8, functions: &[&str]) -> PinSpec {
    PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: Some(gpio), functions: functions.iter().map(|x| x.to_string()).collect() }
}

pub(super) fn power(number: u8, name: &str, kind: PinKind) -> PinSpec {
    PinSpec { number, name: name.into(), kind, gpio: None, functions: vec![] }
}

/// Dedicated pin without a GPIO slot (RESET, XTAL1, XTAL2).
pub(super) fn dedicated(number: u8, name: &str) -> PinSpec {
    PinSpec { number, name: name.into(), kind: PinKind::Io, gpio: None, functions: vec![name.to_string()] }
}
