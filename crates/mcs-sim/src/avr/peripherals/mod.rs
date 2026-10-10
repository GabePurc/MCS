//! Peripheral models and per-family wiring recipes. Each device spec names a
//! `PeripheralSet`; the recipe instantiates reusable models with that device's register
//! addresses, vectors and pin mapping.

pub mod analog;
pub mod classic;
pub mod eeprom;
pub mod exint;
pub mod irqflags;
pub mod port;
pub mod serial;
pub mod spi;
pub mod stimulus;
pub mod system;
pub mod timer;
pub mod timer1hs;
pub mod twi;
pub mod usart;
pub mod usi;

use mcs_core::avr::device::{PeripheralSet, SleepKind};

use super::machine::{Machine, Peripheral, Trigger};
use analog::{AcConfig, AcmeConfig, Adc, AdcConfig, AdcInput, AdcRef, AnalogComparator, BANDGAP_V, TINY10_TRIGGERS};
use classic::{ClassicSystem, ClassicSystemConfig, ClockSource};
use eeprom::{Eeprom, EepromConfig};
use exint::{ExtInt, ExtIntConfig, IntSpec, PcGroupSpec};
use irqflags::{Gtccr, GtccrConfig, IrqFlags, IrqFlagsConfig};
use port::{Port, PortConfig};
use spi::{Spi, SpiConfig};
use system::{System, SystemConfig, Watchdog, WatchdogConfig};
use timer::{Timer, TimerBits, TimerConfig, CS_SYNC, CS_TIMER2};
use timer1hs::{Timer1Hs, Timer1HsConfig};
use twi::{Twi, TwiConfig};
use usart::{Usart, UsartConfig};
use usi::{Usi, UsiConfig};

pub fn wire(m: &mut Machine) {
    match m.spec.peripheral_set {
        PeripheralSet::TinyRc => wire_tiny_rc(m),
        PeripheralSet::MegaX8 => wire_mega_x8(m),
        PeripheralSet::TinyX5 => wire_tiny_x5(m),
        PeripheralSet::Tiny13 => wire_tiny13(m),
        PeripheralSet::TinyX4 => wire_tiny_x4(m),
        PeripheralSet::TinyX313 => wire_tiny_x313(m),
        PeripheralSet::Custom => wire_custom(m),
    }
    // Test bench (every device): signal generators and the Serial Monitor's serial port.
    let pins = m.sys.pins.len();
    m.stimulus = Some(m.add_peripheral(Box::new(stimulus::Stimulus::new(pins))));
    m.serial = Some(m.add_peripheral(Box::new(serial::SerialBridge::new())));
}

fn add(m: &mut Machine, p: Box<dyn Peripheral>, regs: Vec<(u16, u8)>, vectors: &[Option<u8>]) -> u8 {
    let idx = m.add_peripheral(p);
    for (addr, rmw) in regs {
        m.claim_io(addr, idx, rmw);
    }
    for &v in vectors {
        m.claim_irq(v, idx);
    }
    idx
}

/// Adds a port; PINx gets the SBI/CBI toggle-one-bit semantics.
fn add_port(m: &mut Machine, c: PortConfig) {
    let port = Port::new(c);
    let mut regs: Vec<(u16, u8)> = port.registers().into_iter().map(|a| (a, 0)).collect();
    regs[0].1 = 0xff; // SBI/CBI on PINx only toggle the addressed bit
    add(m, Box::new(port), regs, &[]);
}

/// Adds timers sharing one TIFR/TIMSK pair plus the pair's owner.
fn add_timers(m: &mut Machine, name: &'static str, tifr: u16, timsk: u16, timers: Vec<Box<dyn Peripheral>>, regs: Vec<Vec<(u16, u8)>>, map: Vec<(u8, u8)>) {
    for (t, r) in timers.into_iter().zip(regs) {
        add(m, t, r, &[]);
    }
    let flags = IrqFlags::new(IrqFlagsConfig { name, flag_reg: tifr, mask_reg: timsk, map });
    let (regs, vecs) = (flags.registers(), flags.vectors());
    add(m, Box::new(flags), regs, &vecs);
}

fn add_timer(m: &mut Machine, c: TimerConfig) {
    let (tifr, timsk, name) = (c.tifr, c.timsk, c.name);
    let t = Timer::new(c);
    let (regs, map) = (t.registers(), t.irq_map());
    add_timers(m, name, tifr, timsk, vec![Box::new(t)], vec![regs], map);
}

/// Wake-up vectors per canonical sleep mode.
fn set_wake(m: &mut Machine, table: &[(SleepKind, &[&str])]) {
    let n = m.cpu.vector_count;
    let mut masks = vec![vec![true; n]; SleepKind::COUNT];
    for (kind, names) in table {
        let mut a = vec![false; n];
        for name in *names {
            if let Some(i) = m.spec.vector(name) {
                a[i as usize] = true;
            }
        }
        masks[*kind as usize] = a;
    }
    m.cpu.wake_mask = masks;
}

const ALL_SLEEP: u8 = 0;

/// ATtiny4/5/9/10.
fn wire_tiny_rc(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n);

    // System first: its reset handler sets RSTFLR before the watchdog reads WDRF.
    let sys = System::new(SystemConfig {
        ccp: r("CCP"), clkmsr: r("CLKMSR"), clkpsr: r("CLKPSR"), osccal: r("OSCCAL"), smcr: r("SMCR"), rstflr: r("RSTFLR"),
        prr: r("PRR"), vlmcsr: r("VLMCSR"), vlm_vector: v("VLM").unwrap(), nvmcsr: r("NVMCSR"), nvmcmd: r("NVMCMD"),
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[v("VLM")]);

    add_port(m, PortConfig {
        name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: Some(r("PUEB")),
        didr: (0..4).map(|i| Some((r("DIDR0"), 1u8 << i))).collect(), pud: None,
        gpios: vec![0, 1, 2, 3], reset_gpio: Some(3),
    });

    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![IntSpec { gpio: 2, vector: v("INT0").unwrap(), isc_reg: r("EICRA"), isc_shift: 0, mask_reg: r("EIMSK"), mask_bit: 1, flag_reg: r("EIFR"), flag_bit: 1 }],
        groups: vec![PcGroupSpec { gpios: vec![0, 1, 2, 3], msk_reg: r("PCMSK"), vector: v("PCINT0").unwrap(), enable_reg: r("PCICR"), enable_bit: 1, flag_reg: r("PCIFR"), flag_bit: 1 }],
        owned: vec![(r("EICRA"), 0x03, false), (r("EIMSK"), 0x01, false), (r("EIFR"), 0, true), (r("PCICR"), 0x01, false), (r("PCIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    add_timer(m, TimerConfig {
        name: "TC0", id: 0, wide: true,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0C"), tcnt: r("TCNT0L"), ocr_a: r("OCR0AL"), ocr_b: r("OCR0BL"), icr: Some(r("ICR0L")),
        tifr: r("TIFR0"), timsk: r("TIMSK0"), bits: TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: 0x20 },
        v_ovf: v("TIM0_OVF").unwrap(), v_comp_a: v("TIM0_COMPA").unwrap(), v_comp_b: v("TIM0_COMPB").unwrap(), v_capt: v("TIM0_CAPT"),
        oc_a_gpio: Some(0), oc_b_gpio: Some(1), icp_gpio: Some(1), t_gpio: Some(2), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x01, sleep_run: ALL_SLEEP,
    });
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr: vec![(0x01, 1)], strobes: 0, config: 0 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let ac = AnalogComparator::new(AcConfig { acsr: r("ACSR"), ain0_gpio: 0, ain1_gpio: 1, vector: v("ANA_COMP").unwrap(), acbg: false, acic: true, acme: None });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[v("ANA_COMP")]);

    if s.has_adc {
        let adc = Adc::new(AdcConfig {
            adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: None,
            mux_mask: 0x03, inputs: (0..4).map(|g| Some(AdcInput::Pin(g))).collect(),
            ref_mask: 0, ref_extra: 0, refs: vec![Some(AdcRef::Vcc)], adlar: 0, adlar_srb: false, admux_mask: 0x03, adcsrb_mask: 0x07, bin: 0,
            triggers: TINY10_TRIGGERS, vector: v("ADC").unwrap(), prr_mask: 0x02, notify: false,
        });
        let regs = adc.registers();
        add(m, Box::new(adc), regs, &[v("ADC")]);
    }

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("RSTFLR"), vector: v("WDT").unwrap(), wdce: false });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[v("WDT")]);

    // Wake-up sources per sleep mode (datasheet table 7-1).
    let deep: &[&str] = &["INT0", "PCINT0", "WDT"];
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &["INT0", "PCINT0", "ADC", "WDT", "VLM"]),
        (SleepKind::PowerDown, deep),
        (SleepKind::PowerSave, deep),
        (SleepKind::Standby, deep),
        (SleepKind::ExtendedStandby, deep),
    ]);
}

/// Bandgap and temperature sensor readings at 25 °C (DS40002061B table 24-2, Atmel-2586Q 17.12).
const MEGA_TEMP_V: f64 = 0.314;
const TINY85_TEMP_V: f64 = 300.0 / 1024.0 * BANDGAP_V;

/// ATmega48PA/88PA/168PA/328P.
fn wire_mega_x8(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap();
    // GPIO numbering: PB0-7 = 0-7, PC0-6 = 8-14, PD0-7 = 15-22.
    const PB: usize = 0;
    const PC: usize = 8;
    const PD: usize = 15;

    let crystal = |k: u8| (k, ClockSource::Crystal);
    let mut cksel = vec![(0, ClockSource::External), (2, ClockSource::Rc8M), (3, ClockSource::Rc128k), (4, ClockSource::LowFreqCrystal), (5, ClockSource::LowFreqCrystal), crystal(6), crystal(7)];
    cksel.extend((8..16).map(crystal));
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"), mcucr_plain: 0x10,
        ivsel: s.boot.as_ref().map(|_| (0x02, 0x01)), bods: Some((0x40, 0x20)),
        prr: r("PRR"), prr_mask: 0xef, osccal: r("OSCCAL"), pllcsr: None, cksel,
        xtal1: Some(PB + 6), xtal2: Some(PB + 7),
        bod_levels: vec![(6, 1.8), (5, 2.7), (4, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    let pud = Some((r("MCUCR"), 0x10));
    add_port(m, PortConfig { name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: None, didr: vec![], pud, gpios: (PB..PB + 8).collect(), reset_gpio: None });
    add_port(m, PortConfig {
        name: "PORTC", pin: r("PINC"), ddr: r("DDRC"), port: r("PORTC"), pue: None,
        didr: (0..6).map(|i| Some((r("DIDR0"), 1u8 << i))).collect(), pud, gpios: (PC..PC + 7).collect(), reset_gpio: Some(PC + 6),
    });
    add_port(m, PortConfig {
        name: "PORTD", pin: r("PIND"), ddr: r("DDRD"), port: r("PORTD"), pue: None,
        didr: vec![None, None, None, None, None, None, Some((r("DIDR1"), 0x01)), Some((r("DIDR1"), 0x02))], pud, gpios: (PD..PD + 8).collect(), reset_gpio: None,
    });

    let int = |gpio: usize, vector: u8, n: u8| IntSpec { gpio: gpio as u8, vector, isc_reg: r("EICRA"), isc_shift: n * 2, mask_reg: r("EIMSK"), mask_bit: 1 << n, flag_reg: r("EIFR"), flag_bit: 1 << n };
    let group = |base: usize, count: usize, msk: &str, vector: u8, n: u8| PcGroupSpec {
        gpios: (base..base + count).map(|g| g as u8).collect(), msk_reg: r(msk), vector, enable_reg: r("PCICR"), enable_bit: 1 << n, flag_reg: r("PCIFR"), flag_bit: 1 << n,
    };
    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![int(PD + 2, v("INT0"), 0), int(PD + 3, v("INT1"), 1)],
        groups: vec![group(PB, 8, "PCMSK0", v("PCINT0"), 0), group(PC, 7, "PCMSK1", v("PCINT1"), 1), group(PD, 8, "PCMSK2", v("PCINT2"), 2)],
        owned: vec![(r("EICRA"), 0x0f, false), (r("EIMSK"), 0x03, false), (r("EIFR"), 0, true), (r("PCICR"), 0x07, false), (r("PCIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    let bits8 = TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: 0 };
    add_timer(m, TimerConfig {
        name: "TC0", id: 0, wide: false,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0B"), tcnt: r("TCNT0"), ocr_a: r("OCR0A"), ocr_b: r("OCR0B"), icr: None,
        tifr: r("TIFR0"), timsk: r("TIMSK0"), bits: bits8,
        v_ovf: v("TIMER0_OVF"), v_comp_a: v("TIMER0_COMPA"), v_comp_b: v("TIMER0_COMPB"), v_capt: None,
        oc_a_gpio: Some(PD + 6), oc_b_gpio: Some(PD + 5), icp_gpio: None, t_gpio: Some((PD + 4) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x20, sleep_run: ALL_SLEEP,
    });
    add_timer(m, TimerConfig {
        name: "TC1", id: 1, wide: true,
        tccr_a: r("TCCR1A"), tccr_b: r("TCCR1B"), foc_reg: r("TCCR1C"), tcnt: r("TCNT1L"), ocr_a: r("OCR1AL"), ocr_b: r("OCR1BL"), icr: Some(r("ICR1L")),
        tifr: r("TIFR1"), timsk: r("TIMSK1"), bits: TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: 0x20 },
        v_ovf: v("TIMER1_OVF"), v_comp_a: v("TIMER1_COMPA"), v_comp_b: v("TIMER1_COMPB"), v_capt: Some(v("TIMER1_CAPT")),
        oc_a_gpio: Some(PB + 1), oc_b_gpio: Some(PB + 2), icp_gpio: Some(PB as u8), t_gpio: Some((PD + 5) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x08, sleep_run: ALL_SLEEP,
    });
    add_timer(m, TimerConfig {
        name: "TC2", id: 2, wide: false,
        tccr_a: r("TCCR2A"), tccr_b: r("TCCR2B"), foc_reg: r("TCCR2B"), tcnt: r("TCNT2"), ocr_a: r("OCR2A"), ocr_b: r("OCR2B"), icr: None,
        tifr: r("TIFR2"), timsk: r("TIMSK2"), bits: bits8,
        v_ovf: v("TIMER2_OVF"), v_comp_a: v("TIMER2_COMPA"), v_comp_b: v("TIMER2_COMPB"), v_capt: None,
        oc_a_gpio: Some(PB + 3), oc_b_gpio: Some(PD + 3), icp_gpio: None, t_gpio: None, clock: CS_TIMER2, prescaler_group: 2, prr_mask: 0x40,
        // Timer2 keeps running in power-save and extended standby (DS40002061B 10.6).
        sleep_run: (1 << SleepKind::PowerSave as u8) | (1 << SleepKind::ExtendedStandby as u8),
    });
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr: vec![(0x01, 1), (0x02, 2)], strobes: 0, config: 0 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let usart = Usart::new(UsartConfig {
        name: "USART0", udr: r("UDR0"), ucsra: r("UCSR0A"), ucsrb: r("UCSR0B"), ucsrc: r("UCSR0C"), ubrrl: r("UBRR0L"), ubrrh: r("UBRR0H"),
        rx_gpio: PD, tx_gpio: PD + 1, v_rx: v("USART_RX"), v_udre: v("USART_UDRE"), v_tx: v("USART_TX"), prr_mask: 0x02,
    });
    let (regs, vecs) = (usart.registers(), usart.vectors());
    add(m, Box::new(usart), regs, &vecs);

    let spi = Spi::new(SpiConfig {
        spcr: r("SPCR"), spsr: r("SPSR"), spdr: r("SPDR"), ss_gpio: PB + 2, mosi_gpio: PB + 3, miso_gpio: PB + 4, sck_gpio: PB + 5, vector: v("SPI_STC"), prr_mask: 0x04,
    });
    let regs = spi.registers();
    add(m, Box::new(spi), regs, &[Some(v("SPI_STC"))]);

    let twi = Twi::new(TwiConfig { twbr: r("TWBR"), twsr: r("TWSR"), twar: r("TWAR"), twdr: r("TWDR"), twcr: r("TWCR"), twamr: r("TWAMR"), vector: v("TWI"), prr_mask: 0x80 });
    let regs = twi.registers();
    add(m, Box::new(twi), regs, &[Some(v("TWI"))]);

    let ac = AnalogComparator::new(AcConfig {
        acsr: r("ACSR"), ain0_gpio: (PD + 6) as u8, ain1_gpio: (PD + 7) as u8, vector: v("ANALOG_COMP"), acbg: true, acic: true,
        acme: Some(AcmeConfig {
            reg: r("ADCSRB"), bit: 0x40, adcsra: r("ADCSRA"), admux: r("ADMUX"), mux_mask: 0x07,
            channels: (0..8).map(|i| (i < 6).then_some(PC + i)).collect(),
        }),
    });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[Some(v("ANALOG_COMP"))]);

    let mut inputs: Vec<Option<AdcInput>> = (0..16).map(|i| (i < 6).then_some(AdcInput::Pin(PC + i))).collect();
    inputs[8] = Some(AdcInput::Temp(MEGA_TEMP_V));
    inputs[14] = Some(AdcInput::Volts(BANDGAP_V));
    inputs[15] = Some(AdcInput::Volts(0.0));
    let adc = Adc::new(AdcConfig {
        adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: Some(r("ADCH")), mux_mask: 0x0f, inputs,
        // REFS1:0 = 00 AREF (tied to VCC here), 01 AVCC, 11 internal 1.1 V.
        ref_mask: 0xc0, ref_extra: 0, refs: vec![Some(AdcRef::Aref(None)), Some(AdcRef::Vcc), None, Some(AdcRef::Volts(BANDGAP_V))],
        adlar: 0x20, adlar_srb: false, admux_mask: 0xef, adcsrb_mask: 0x47, bin: 0,
        triggers: [None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::TimerCompA(0)), Some(Trigger::TimerOvf(0)), Some(Trigger::TimerCompB(1)), Some(Trigger::TimerOvf(1)), Some(Trigger::TimerCapt(1))],
        vector: v("ADC"), prr_mask: 0x01, notify: true,
    });
    let regs = adc.registers();
    add(m, Box::new(adc), regs, &[Some(v("ADC"))]);

    let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEARL"), eearh: Some(r("EEARH")), vector: v("EE_READY") });
    let regs = ee.registers();
    add(m, Box::new(ee), regs, &[Some(v("EE_READY"))]);

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (DS40002061B table 10-1).
    let pd: &[&str] = &["INT0", "INT1", "PCINT0", "PCINT1", "PCINT2", "TWI", "WDT"];
    let ps: &[&str] = &["INT0", "INT1", "PCINT0", "PCINT1", "PCINT2", "TWI", "WDT", "TIMER2_COMPA", "TIMER2_COMPB", "TIMER2_OVF"];
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &["INT0", "INT1", "PCINT0", "PCINT1", "PCINT2", "TWI", "TIMER2_COMPA", "TIMER2_COMPB", "TIMER2_OVF", "SPM_READY", "EE_READY", "ADC", "WDT"]),
        (SleepKind::PowerDown, pd),
        (SleepKind::PowerSave, ps),
        (SleepKind::Standby, pd),
        (SleepKind::ExtendedStandby, ps),
    ]);
}

/// ATtiny25/45/85.
fn wire_tiny_x5(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap();

    let crystal = |k: u8| (k, ClockSource::Crystal);
    let mut cksel = vec![(0, ClockSource::External), (1, ClockSource::Pll16M), (2, ClockSource::Rc8M), (3, ClockSource::Rc6M4), (4, ClockSource::Rc128k), (6, ClockSource::LowFreqCrystal)];
    cksel.extend((8..16).map(crystal));
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"),
        // PUD, SE, SM1:0 and ISC01:00 are plain bits (sleep and INT0 read them).
        mcucr_plain: 0x7b, ivsel: None, bods: Some((0x80, 0x04)),
        prr: r("PRR"), prr_mask: 0x0f, osccal: r("OSCCAL"), pllcsr: Some(r("PLLCSR")), cksel,
        xtal1: Some(3), xtal2: Some(4),
        bod_levels: vec![(6, 1.8), (5, 2.7), (4, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    let didr = r("DIDR0");
    add_port(m, PortConfig {
        name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: None,
        // DIDR0: AIN0D (PB0), AIN1D (PB1), ADC1D (PB2), ADC3D (PB3), ADC2D (PB4), ADC0D (PB5).
        didr: [0x01, 0x02, 0x04, 0x08, 0x10, 0x20].iter().map(|&b| Some((didr, b))).collect(),
        pud: Some((r("MCUCR"), 0x40)), gpios: (0..6).collect(), reset_gpio: Some(5),
    });

    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![IntSpec { gpio: 2, vector: v("INT0"), isc_reg: r("MCUCR"), isc_shift: 0, mask_reg: r("GIMSK"), mask_bit: 0x40, flag_reg: r("GIFR"), flag_bit: 0x40 }],
        groups: vec![PcGroupSpec { gpios: (0..6).collect(), msk_reg: r("PCMSK"), vector: v("PCINT0"), enable_reg: r("GIMSK"), enable_bit: 0x20, flag_reg: r("GIFR"), flag_bit: 0x20 }],
        owned: vec![(r("GIMSK"), 0x60, false), (r("GIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    // Timer0 and Timer1 share TIFR/TIMSK.
    let t0 = Timer::new(TimerConfig {
        name: "TC0", id: 0, wide: false,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0B"), tcnt: r("TCNT0"), ocr_a: r("OCR0A"), ocr_b: r("OCR0B"), icr: None,
        tifr: r("TIFR"), timsk: r("TIMSK"), bits: TimerBits { tov: 0x02, ocfa: 0x10, ocfb: 0x08, icf: 0 },
        v_ovf: v("TIM0_OVF"), v_comp_a: v("TIM0_COMPA"), v_comp_b: v("TIM0_COMPB"), v_capt: None,
        oc_a_gpio: Some(0), oc_b_gpio: Some(1), icp_gpio: None, t_gpio: Some(2), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x04, sleep_run: ALL_SLEEP,
    });
    let t1 = Timer1Hs::new(Timer1HsConfig {
        tccr1: r("TCCR1"), gtccr: r("GTCCR"), tcnt1: r("TCNT1"), ocr1a: r("OCR1A"), ocr1b: r("OCR1B"), ocr1c: r("OCR1C"), pllcsr: r("PLLCSR"),
        tifr: r("TIFR"), timsk: r("TIMSK"), ocfa: 0x40, ocfb: 0x20, tov: 0x04,
        v_comp_a: v("TIM1_COMPA"), v_comp_b: v("TIM1_COMPB"), v_ovf: v("TIM1_OVF"),
        oc: [1, 0, 4, 3], prescaler_group: 2, prr_mask: 0x08,
    });
    let mut map = t0.irq_map();
    map.extend(t1.irq_map());
    let regs = vec![t0.registers(), t1.registers()];
    add_timers(m, "TIMERS", r("TIFR"), r("TIMSK"), vec![Box::new(t0), Box::new(t1)], regs, map);
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr: vec![(0x01, 1), (0x02, 2)], strobes: 0x0c, config: 0x70 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let usi = Usi::new(UsiConfig {
        usicr: r("USICR"), usisr: r("USISR"), usidr: r("USIDR"), usibr: r("USIBR"), port: r("PORTB"),
        di_gpio: 0, do_gpio: 1, usck_gpio: 2, usck_bit: 0x04, v_start: v("USI_START"), v_ovf: v("USI_OVF"), prr_mask: 0x02,
    });
    let regs = usi.registers();
    add(m, Box::new(usi), regs, &[Some(v("USI_START")), Some(v("USI_OVF"))]);

    let ac = AnalogComparator::new(AcConfig {
        acsr: r("ACSR"), ain0_gpio: 0, ain1_gpio: 1, vector: v("ANA_COMP"), acbg: true, acic: false,
        acme: Some(AcmeConfig { reg: r("ADCSRB"), bit: 0x40, adcsra: r("ADCSRA"), admux: r("ADMUX"), mux_mask: 0x03, channels: vec![Some(5), Some(2), Some(4), Some(3)] }),
    });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[Some(v("ANA_COMP"))]);

    // ADC0 = PB5, ADC1 = PB2, ADC2 = PB4, ADC3 = PB3 (Atmel-2586Q table 17-4).
    let p = AdcInput::Pin;
    let d = AdcInput::Diff;
    let inputs = vec![
        Some(p(5)), Some(p(2)), Some(p(4)), Some(p(3)),
        Some(d(4, 4, 1.0)), Some(d(4, 4, 20.0)), Some(d(4, 3, 1.0)), Some(d(4, 3, 20.0)),
        Some(d(5, 5, 1.0)), Some(d(5, 5, 20.0)), Some(d(5, 2, 1.0)), Some(d(5, 2, 20.0)),
        Some(AdcInput::Volts(BANDGAP_V)), Some(AdcInput::Volts(0.0)), None, Some(AdcInput::Temp(TINY85_TEMP_V)),
    ];
    let adc = Adc::new(AdcConfig {
        adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: Some(r("ADCH")), mux_mask: 0x0f, inputs,
        // REFS2:0 = 000 VCC, 001 AREF (PB0), 010 1.1 V, 110/111 2.56 V.
        ref_mask: 0xc0, ref_extra: 0x10,
        refs: vec![Some(AdcRef::Vcc), Some(AdcRef::Aref(Some(0))), Some(AdcRef::Volts(BANDGAP_V)), None, None, None, Some(AdcRef::Volts(2.56)), Some(AdcRef::Volts(2.56))],
        adlar: 0x20, adlar_srb: false, admux_mask: 0xff, adcsrb_mask: 0xe7, bin: 0x80,
        triggers: [None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::TimerCompA(0)), Some(Trigger::TimerOvf(0)), Some(Trigger::TimerCompB(0)), Some(Trigger::PcInt), None],
        vector: v("ADC"), prr_mask: 0x01, notify: true,
    });
    let regs = adc.registers();
    add(m, Box::new(adc), regs, &[Some(v("ADC"))]);

    let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEARL"), eearh: Some(r("EEARH")), vector: v("EE_RDY") });
    let regs = ee.registers();
    add(m, Box::new(ee), regs, &[Some(v("EE_RDY"))]);

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (Atmel-2586Q table 7-1).
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &["INT0", "PCINT0", "USI_START", "EE_RDY", "ADC", "WDT"]),
        (SleepKind::PowerDown, &["INT0", "PCINT0", "USI_START", "WDT"]),
    ]);
}

/// ATtiny13A.
fn wire_tiny13(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap();

    // CKSEL1:0 = 00 external clock, 01 internal RC at 4.8 MHz, 10 at 9.6 MHz, 11 128 kHz.
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"), mcucr_plain: 0x7b, ivsel: None, bods: None,
        prr: r("PRR"), prr_mask: 0x03, osccal: r("OSCCAL"), pllcsr: None,
        cksel: vec![(0, ClockSource::External), (1, ClockSource::RcHalf), (2, ClockSource::Rc8M), (3, ClockSource::Rc128k)],
        xtal1: Some(3), xtal2: None,
        // BODLEVEL1:0 = 11 disabled, 10 1.8 V, 01 2.7 V, 00 4.3 V.
        bod_levels: vec![(2, 1.8), (1, 2.7), (0, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    let didr = r("DIDR0");
    add_port(m, PortConfig {
        name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: None,
        // DIDR0: AIN0D (PB0), AIN1D (PB1), ADC1D (PB2), ADC3D (PB3), ADC2D (PB4), ADC0D (PB5).
        didr: [0x01, 0x02, 0x04, 0x08, 0x10, 0x20].iter().map(|&b| Some((didr, b))).collect(),
        pud: Some((r("MCUCR"), 0x40)), gpios: (0..6).collect(), reset_gpio: Some(5),
    });

    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![IntSpec { gpio: 1, vector: v("INT0"), isc_reg: r("MCUCR"), isc_shift: 0, mask_reg: r("GIMSK"), mask_bit: 0x40, flag_reg: r("GIFR"), flag_bit: 0x40 }],
        groups: vec![PcGroupSpec { gpios: (0..6).collect(), msk_reg: r("PCMSK"), vector: v("PCINT0"), enable_reg: r("GIMSK"), enable_bit: 0x20, flag_reg: r("GIFR"), flag_bit: 0x20 }],
        owned: vec![(r("GIMSK"), 0x60, false), (r("GIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    add_timer(m, TimerConfig {
        name: "TC0", id: 0, wide: false,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0B"), tcnt: r("TCNT0"), ocr_a: r("OCR0A"), ocr_b: r("OCR0B"), icr: None,
        tifr: r("TIFR0"), timsk: r("TIMSK0"), bits: TimerBits { tov: 0x02, ocfa: 0x04, ocfb: 0x08, icf: 0 },
        v_ovf: v("TIM0_OVF"), v_comp_a: v("TIM0_COMPA"), v_comp_b: v("TIM0_COMPB"), v_capt: None,
        oc_a_gpio: Some(0), oc_b_gpio: Some(1), icp_gpio: None, t_gpio: Some(2), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x02, sleep_run: ALL_SLEEP,
    });
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr: vec![(0x01, 1)], strobes: 0, config: 0 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let ac = AnalogComparator::new(AcConfig {
        acsr: r("ACSR"), ain0_gpio: 0, ain1_gpio: 1, vector: v("ANA_COMP"), acbg: true, acic: false,
        acme: Some(AcmeConfig { reg: r("ADCSRB"), bit: 0x40, adcsra: r("ADCSRA"), admux: r("ADMUX"), mux_mask: 0x03, channels: vec![Some(5), Some(2), Some(4), Some(3)] }),
    });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[Some(v("ANA_COMP"))]);

    // ADC0 = PB5, ADC1 = PB2, ADC2 = PB4, ADC3 = PB3; REFS0: 0 VCC, 1 internal 1.1 V.
    let adc = Adc::new(AdcConfig {
        adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: Some(r("ADCH")), mux_mask: 0x03,
        inputs: [5, 2, 4, 3].iter().map(|&g| Some(AdcInput::Pin(g))).collect(),
        ref_mask: 0x40, ref_extra: 0, refs: vec![Some(AdcRef::Vcc), Some(AdcRef::Volts(BANDGAP_V))],
        adlar: 0x20, adlar_srb: false, admux_mask: 0x63, adcsrb_mask: 0x47, bin: 0,
        triggers: [None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::TimerCompA(0)), Some(Trigger::TimerOvf(0)), Some(Trigger::TimerCompB(0)), Some(Trigger::PcInt), None],
        vector: v("ADC"), prr_mask: 0x01, notify: true,
    });
    let regs = adc.registers();
    add(m, Box::new(adc), regs, &[Some(v("ADC"))]);

    let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEARL"), eearh: None, vector: v("EE_RDY") });
    let regs = ee.registers();
    add(m, Box::new(ee), regs, &[Some(v("EE_RDY"))]);

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (ATtiny13A table 7-1).
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &["INT0", "PCINT0", "EE_RDY", "ADC", "WDT"]),
        (SleepKind::PowerDown, &["INT0", "PCINT0", "WDT"]),
    ]);
}

/// ADC input table of the ATtiny24A/44A/84A (Atmel-8183F tables 18-4 / 18-5): single-ended ADC0-7,
/// differential pairs at 1x (MUX0 = 0) / 20x (MUX0 = 1) with MUX5 reversing the polarity,
/// offset-calibration channels, GND, 1.1 V and the temperature sensor (ADC8).
fn tiny_x4_adc_inputs() -> Vec<Option<AdcInput>> {
    let mut inputs: Vec<Option<AdcInput>> = vec![None; 64];
    for (g, slot) in inputs.iter_mut().enumerate().take(8) {
        *slot = Some(AdcInput::Pin(g));
    }
    // (MUX5:0 for 1x gain, positive ADC, negative ADC); the 20x code is the 1x code + 1.
    const PAIRS: [(usize, usize, usize); 24] = [
        (0b001000, 0, 1), (0b001010, 0, 3), (0b101000, 1, 0), (0b001100, 1, 2), (0b001110, 1, 3), (0b101100, 2, 1),
        (0b010000, 2, 3), (0b101010, 3, 0), (0b101110, 3, 1), (0b110000, 3, 2), (0b010010, 3, 4), (0b010100, 3, 5),
        (0b010110, 3, 6), (0b011000, 3, 7), (0b110010, 4, 3), (0b011010, 4, 5), (0b110100, 5, 3), (0b111010, 5, 4),
        (0b011100, 5, 6), (0b110110, 6, 3), (0b111100, 6, 5), (0b011110, 6, 7), (0b111000, 7, 3), (0b111110, 7, 6),
    ];
    for (code, p, n) in PAIRS {
        inputs[code] = Some(AdcInput::Diff(p, n, 1.0));
        inputs[code + 1] = Some(AdcInput::Diff(p, n, 20.0));
    }
    // Offset calibration: both inputs on the same pin (ADC0 only at 20x).
    inputs[0b100011] = Some(AdcInput::Diff(0, 0, 20.0));
    for (code, g) in [(0b100100, 3), (0b100110, 7)] {
        inputs[code] = Some(AdcInput::Diff(g, g, 1.0));
        inputs[code + 1] = Some(AdcInput::Diff(g, g, 20.0));
    }
    inputs[0b100000] = Some(AdcInput::Volts(0.0));
    inputs[0b100001] = Some(AdcInput::Volts(BANDGAP_V));
    inputs[0b100010] = Some(AdcInput::Temp(MEGA_TEMP_V));
    inputs
}

/// ATtiny24A/44A/84A.
fn wire_tiny_x4(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap();
    // GPIO numbering: PA0-7 = 0-7, PB0-3 = 8-11.
    const PA: usize = 0;
    const PB: usize = 8;

    let crystal = |k: u8| (k, ClockSource::Crystal);
    let mut cksel = vec![(0, ClockSource::External), (2, ClockSource::Rc8M), (4, ClockSource::Rc128k), (6, ClockSource::LowFreqCrystal)];
    cksel.extend((8..16).map(crystal));
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"), mcucr_plain: 0x7b, ivsel: None, bods: Some((0x80, 0x04)),
        prr: r("PRR"), prr_mask: 0x0f, osccal: r("OSCCAL"), pllcsr: None, cksel,
        xtal1: Some(PB), xtal2: Some(PB + 1),
        bod_levels: vec![(6, 1.8), (5, 2.7), (4, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    let pud = Some((r("MCUCR"), 0x40));
    add_port(m, PortConfig {
        name: "PORTA", pin: r("PINA"), ddr: r("DDRA"), port: r("PORTA"), pue: None,
        didr: (0..8).map(|i| Some((r("DIDR0"), 1u8 << i))).collect(), pud, gpios: (PA..PA + 8).collect(), reset_gpio: None,
    });
    add_port(m, PortConfig { name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: None, didr: vec![], pud, gpios: (PB..PB + 4).collect(), reset_gpio: Some(PB + 3) });

    let group = |base: usize, count: usize, msk: &str, vector: u8, bit: u8| PcGroupSpec {
        gpios: (base..base + count).map(|g| g as u8).collect(), msk_reg: r(msk), vector, enable_reg: r("GIMSK"), enable_bit: bit, flag_reg: r("GIFR"), flag_bit: bit,
    };
    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![IntSpec { gpio: (PB + 2) as u8, vector: v("INT0"), isc_reg: r("MCUCR"), isc_shift: 0, mask_reg: r("GIMSK"), mask_bit: 0x40, flag_reg: r("GIFR"), flag_bit: 0x40 }],
        groups: vec![group(PA, 8, "PCMSK0", v("PCINT0"), 0x10), group(PB, 4, "PCMSK1", v("PCINT1"), 0x20)],
        owned: vec![(r("GIMSK"), 0x70, false), (r("GIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    add_timer(m, TimerConfig {
        name: "TC0", id: 0, wide: false,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0B"), tcnt: r("TCNT0"), ocr_a: r("OCR0A"), ocr_b: r("OCR0B"), icr: None,
        tifr: r("TIFR0"), timsk: r("TIMSK0"), bits: TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: 0 },
        v_ovf: v("TIM0_OVF"), v_comp_a: v("TIM0_COMPA"), v_comp_b: v("TIM0_COMPB"), v_capt: None,
        oc_a_gpio: Some(PB + 2), oc_b_gpio: Some(PA + 7), icp_gpio: None, t_gpio: Some((PA + 3) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x04, sleep_run: ALL_SLEEP,
    });
    add_timer(m, TimerConfig {
        name: "TC1", id: 1, wide: true,
        tccr_a: r("TCCR1A"), tccr_b: r("TCCR1B"), foc_reg: r("TCCR1C"), tcnt: r("TCNT1L"), ocr_a: r("OCR1AL"), ocr_b: r("OCR1BL"), icr: Some(r("ICR1L")),
        tifr: r("TIFR1"), timsk: r("TIMSK1"), bits: TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: 0x20 },
        v_ovf: v("TIM1_OVF"), v_comp_a: v("TIM1_COMPA"), v_comp_b: v("TIM1_COMPB"), v_capt: Some(v("TIM1_CAPT")),
        oc_a_gpio: Some(PA + 6), oc_b_gpio: Some(PA + 5), icp_gpio: Some((PA + 7) as u8), t_gpio: Some((PA + 4) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x08, sleep_run: ALL_SLEEP,
    });
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr: vec![(0x01, 1)], strobes: 0, config: 0 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let usi = Usi::new(UsiConfig {
        usicr: r("USICR"), usisr: r("USISR"), usidr: r("USIDR"), usibr: r("USIBR"), port: r("PORTA"),
        di_gpio: PA + 6, do_gpio: PA + 5, usck_gpio: PA + 4, usck_bit: 0x10, v_start: v("USI_STR"), v_ovf: v("USI_OVF"), prr_mask: 0x02,
    });
    let regs = usi.registers();
    add(m, Box::new(usi), regs, &[Some(v("USI_STR")), Some(v("USI_OVF"))]);

    let ac = AnalogComparator::new(AcConfig {
        acsr: r("ACSR"), ain0_gpio: (PA + 1) as u8, ain1_gpio: (PA + 2) as u8, vector: v("ANA_COMP"), acbg: true, acic: true,
        acme: Some(AcmeConfig { reg: r("ADCSRB"), bit: 0x40, adcsra: r("ADCSRA"), admux: r("ADMUX"), mux_mask: 0x07, channels: (0..8).map(|i| Some(PA + i)).collect() }),
    });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[Some(v("ANA_COMP"))]);

    let adc = Adc::new(AdcConfig {
        adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: Some(r("ADCH")), mux_mask: 0x3f, inputs: tiny_x4_adc_inputs(),
        // REFS1:0 = 00 VCC, 01 AREF (PA0), 10 internal 1.1 V, 11 reserved.
        ref_mask: 0xc0, ref_extra: 0, refs: vec![Some(AdcRef::Vcc), Some(AdcRef::Aref(Some(PA))), Some(AdcRef::Volts(BANDGAP_V)), None],
        adlar: 0x10, adlar_srb: true, admux_mask: 0xff, adcsrb_mask: 0xd7, bin: 0x80,
        triggers: [None, Some(Trigger::Ac), Some(Trigger::Int0), Some(Trigger::TimerCompA(0)), Some(Trigger::TimerOvf(0)), Some(Trigger::TimerCompB(1)), Some(Trigger::TimerOvf(1)), Some(Trigger::TimerCapt(1))],
        vector: v("ADC"), prr_mask: 0x01, notify: true,
    });
    let regs = adc.registers();
    add(m, Box::new(adc), regs, &[Some(v("ADC"))]);

    let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEARL"), eearh: Some(r("EEARH")), vector: v("EE_RDY") });
    let regs = ee.registers();
    add(m, Box::new(ee), regs, &[Some(v("EE_RDY"))]);

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (Atmel-8183F table 8-1; the USI start detector is asynchronous).
    let deep: &[&str] = &["INT0", "PCINT0", "PCINT1", "USI_STR", "WDT"];
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &["INT0", "PCINT0", "PCINT1", "USI_STR", "EE_RDY", "ADC", "WDT"]),
        (SleepKind::PowerDown, deep),
        (SleepKind::Standby, deep),
    ]);
}

/// ATtiny2313A/4313.
fn wire_tiny_x313(m: &mut Machine) {
    let s = m.spec;
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap();
    // GPIO numbering: PA0-2 = 0-2, PB0-7 = 3-10, PD0-6 = 11-17.
    const PA: usize = 0;
    const PB: usize = 3;
    const PD: usize = 11;

    // CKSEL3:0: 0010 internal 4 MHz, 0100 internal 8 MHz, 0110 128 kHz (Atmel-8246B table 6-1).
    let crystal = |k: u8| (k, ClockSource::Crystal);
    let mut cksel = vec![(0, ClockSource::External), (2, ClockSource::RcHalf), (4, ClockSource::Rc8M), (6, ClockSource::Rc128k)];
    cksel.extend((8..16).map(crystal));
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"), mcucr_plain: 0xff, ivsel: None, bods: None,
        prr: r("PRR"), prr_mask: 0x0f, osccal: r("OSCCAL"), pllcsr: None, cksel,
        xtal1: Some(PA), xtal2: Some(PA + 1),
        bod_levels: vec![(6, 1.8), (5, 2.7), (4, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    let pud = Some((r("MCUCR"), 0x80));
    add_port(m, PortConfig { name: "PORTA", pin: r("PINA"), ddr: r("DDRA"), port: r("PORTA"), pue: None, didr: vec![], pud, gpios: (PA..PA + 3).collect(), reset_gpio: Some(PA + 2) });
    add_port(m, PortConfig {
        name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: None,
        didr: vec![Some((r("DIDR"), 0x01)), Some((r("DIDR"), 0x02))], pud, gpios: (PB..PB + 8).collect(), reset_gpio: None,
    });
    add_port(m, PortConfig { name: "PORTD", pin: r("PIND"), ddr: r("DDRD"), port: r("PORTD"), pue: None, didr: vec![], pud, gpios: (PD..PD + 7).collect(), reset_gpio: None });

    let int = |gpio: usize, vector: u8, n: u8| IntSpec { gpio: gpio as u8, vector, isc_reg: r("MCUCR"), isc_shift: n * 2, mask_reg: r("GIMSK"), mask_bit: 0x40 << n, flag_reg: r("GIFR"), flag_bit: 0x40 << n };
    let group = |base: usize, count: usize, msk: &str, vector: u8, bit: u8| PcGroupSpec {
        gpios: (base..base + count).map(|g| g as u8).collect(), msk_reg: r(msk), vector, enable_reg: r("GIMSK"), enable_bit: bit, flag_reg: r("GIFR"), flag_bit: bit,
    };
    let ext = ExtInt::new(ExtIntConfig {
        ints: vec![int(PD + 2, v("INT0"), 0), int(PD + 3, v("INT1"), 1)],
        groups: vec![group(PB, 8, "PCMSK0", v("PCINT0"), 0x20), group(PA, 3, "PCMSK1", v("PCINT1"), 0x08), group(PD, 7, "PCMSK2", v("PCINT2"), 0x10)],
        owned: vec![(r("GIMSK"), 0xf8, false), (r("GIFR"), 0, true)],
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    // Timer0 and Timer1 share TIFR/TIMSK.
    let t0 = Timer::new(TimerConfig {
        name: "TC0", id: 0, wide: false,
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), foc_reg: r("TCCR0B"), tcnt: r("TCNT0"), ocr_a: r("OCR0A"), ocr_b: r("OCR0B"), icr: None,
        tifr: r("TIFR"), timsk: r("TIMSK"), bits: TimerBits { tov: 0x02, ocfa: 0x01, ocfb: 0x04, icf: 0 },
        v_ovf: v("TIMER0_OVF"), v_comp_a: v("TIMER0_COMPA"), v_comp_b: v("TIMER0_COMPB"), v_capt: None,
        oc_a_gpio: Some(PB + 2), oc_b_gpio: Some(PD + 5), icp_gpio: None, t_gpio: Some((PD + 4) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x04, sleep_run: ALL_SLEEP,
    });
    let t1 = Timer::new(TimerConfig {
        name: "TC1", id: 1, wide: true,
        tccr_a: r("TCCR1A"), tccr_b: r("TCCR1B"), foc_reg: r("TCCR1C"), tcnt: r("TCNT1L"), ocr_a: r("OCR1AL"), ocr_b: r("OCR1BL"), icr: Some(r("ICR1L")),
        tifr: r("TIFR"), timsk: r("TIMSK"), bits: TimerBits { tov: 0x80, ocfa: 0x40, ocfb: 0x20, icf: 0x08 },
        v_ovf: v("TIMER1_OVF"), v_comp_a: v("TIMER1_COMPA"), v_comp_b: v("TIMER1_COMPB"), v_capt: Some(v("TIMER1_CAPT")),
        oc_a_gpio: Some(PB + 3), oc_b_gpio: Some(PB + 4), icp_gpio: Some((PD + 6) as u8), t_gpio: Some((PD + 5) as u8), clock: CS_SYNC, prescaler_group: 1, prr_mask: 0x08, sleep_run: ALL_SLEEP,
    });
    let mut map = t0.irq_map();
    map.extend(t1.irq_map());
    let regs = vec![t0.registers(), t1.registers()];
    add_timers(m, "TIMERS", r("TIFR"), r("TIMSK"), vec![Box::new(t0), Box::new(t1)], regs, map);
    let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0, psr: vec![(0x01, 1)], strobes: 0, config: 0 });
    let regs = g.registers();
    add(m, Box::new(g), regs, &[]);

    let usart = Usart::new(UsartConfig {
        name: "USART0", udr: r("UDR"), ucsra: r("UCSRA"), ucsrb: r("UCSRB"), ucsrc: r("UCSRC"), ubrrl: r("UBRRL"), ubrrh: r("UBRRH"),
        rx_gpio: PD, tx_gpio: PD + 1, v_rx: v("USART_RX"), v_udre: v("USART_UDRE"), v_tx: v("USART_TX"), prr_mask: 0x01,
    });
    let (regs, vecs) = (usart.registers(), usart.vectors());
    add(m, Box::new(usart), regs, &vecs);

    let usi = Usi::new(UsiConfig {
        usicr: r("USICR"), usisr: r("USISR"), usidr: r("USIDR"), usibr: r("USIBR"), port: r("PORTB"),
        di_gpio: PB + 5, do_gpio: PB + 6, usck_gpio: PB + 7, usck_bit: 0x80, v_start: v("USI_START"), v_ovf: v("USI_OVF"), prr_mask: 0x02,
    });
    let regs = usi.registers();
    add(m, Box::new(usi), regs, &[Some(v("USI_START")), Some(v("USI_OVF"))]);

    let ac = AnalogComparator::new(AcConfig {
        acsr: r("ACSR"), ain0_gpio: PB as u8, ain1_gpio: (PB + 1) as u8, vector: v("ANALOG_COMP"), acbg: true, acic: true, acme: None,
    });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[Some(v("ANALOG_COMP"))]);

    let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEAR"), eearh: None, vector: v("EE_READY") });
    let regs = ee.registers();
    add(m, Box::new(ee), regs, &[Some(v("EE_READY"))]);

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (Atmel-8246B table 7-1; the USI start detector is asynchronous).
    let deep: &[&str] = &["INT0", "INT1", "PCINT0", "PCINT1", "PCINT2", "USI_START", "WDT"];
    set_wake(m, &[(SleepKind::PowerDown, deep), (SleepKind::Standby, deep)]);
}

/// Interns a generated peripheral name (models want `&'static str`); bounded by the number of
/// distinct names, so rebuilding machines does not leak.
fn leak(s: String) -> &'static str {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static NAMES: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut g = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    if let Some(&n) = set.get(s.as_str()) {
        return n;
    }
    let n: &'static str = Box::leak(s.into_boxed_str());
    set.insert(n);
    n
}

/// User-defined devices (`mcs_core::avr::devices::custom`): every peripheral that the spec
/// defines is wired by register/pin/vector naming convention, using the same parameterized models
/// as the ATmega recipes. PRR0 gates the instances that have a bit in it (TWI0, TC0-2, SPI0,
/// USART0, ADC); further instances are never power-gated.
fn wire_custom(m: &mut Machine) {
    let s = m.spec;
    let has = |n: &str| s.register(n).is_some();
    let r = |n: &str| s.reg(n);
    let v = |n: &str| s.vector(n).unwrap_or_else(|| panic!("{}: vector {n} not defined", s.name));
    let gpio = |f: &str| s.pins.iter().find(|p| p.gpio.is_some() && p.functions.iter().any(|x| x == f)).and_then(|p| p.gpio);
    let gpio_u = |f: &str| gpio(f).unwrap_or_else(|| panic!("{}: no pin carries {f}", s.name)) as usize;
    let sx = |k: usize| if k == 0 { String::new() } else { k.to_string() };
    let ports = s.gpio_count as usize / 8;
    let prr_bits = s.register("PRR0").map_or(0, |p| p.bits.iter().fold(0u8, |a, b| a | b.mask));

    let crystal = |k: u8| (k, ClockSource::Crystal);
    let mut cksel = vec![(0, ClockSource::External), (2, ClockSource::Rc8M), (3, ClockSource::Rc128k), (4, ClockSource::LowFreqCrystal), (5, ClockSource::LowFreqCrystal), crystal(6), crystal(7)];
    cksel.extend((8..16).map(crystal));
    let sys = ClassicSystem::new(ClassicSystemConfig {
        clkpr: r("CLKPR"), mcusr: r("MCUSR"), mcucr: r("MCUCR"), mcucr_plain: 0x10,
        ivsel: s.boot.as_ref().map(|_| (0x02, 0x01)), bods: Some((0x40, 0x20)),
        prr: r("PRR0"), prr_mask: prr_bits, osccal: r("OSCCAL"), pllcsr: None, cksel,
        xtal1: None, xtal2: None,
        bod_levels: vec![(6, 1.8), (5, 2.7), (4, 4.3)],
    });
    let regs = sys.registers();
    add(m, Box::new(sys), regs, &[]);

    // Ports. The DIDR bit of a pin belongs to the first ADC channel that sits on it.
    let pud = Some((r("MCUCR"), 0x10));
    let adc_ch = (0..32).take_while(|c| gpio(&format!("ADC{c}")).is_some()).count();
    let didr_of = |g: usize| (0..adc_ch).find(|&c| gpio(&format!("ADC{c}")) == Some(g as u8)).map(|c| (r(&mcs_core::avr::devices::didr_name(c)), 1u8 << (c % 8)));
    for p in 0..ports {
        let l = mcs_core::avr::devices::port_name(p);
        let base = p * 8;
        let mut didr: Vec<Option<(u16, u8)>> = (0..8).map(|i| didr_of(base + i)).collect();
        if has("DIDR1") {
            for (f, bit) in [("AIN0", 0x01u8), ("AIN1", 0x02)] {
                if let Some(g) = gpio(f).map(usize::from).filter(|g| (base..base + 8).contains(g)) {
                    if didr[g - base].is_none() {
                        didr[g - base] = Some((r("DIDR1"), bit));
                    }
                }
            }
        }
        while didr.last() == Some(&None) {
            didr.pop();
        }
        add_port(m, PortConfig {
            name: leak(format!("PORT{l}")), pin: r(&format!("PIN{l}")), ddr: r(&format!("DDR{l}")), port: r(&format!("PORT{l}")), pue: None,
            didr, pud, gpios: (base..base + 8).collect(), reset_gpio: None,
        });
    }

    // External and pin-change interrupts.
    let ints = (0..32).take_while(|k| s.vector(&format!("INT{k}")).is_some()).count();
    let eimsk = |k: usize| if k / 8 == 0 { "EIMSK".to_string() } else { format!("EIMSK{}", k / 8) };
    let eifr = |k: usize| if k / 8 == 0 { "EIFR".to_string() } else { format!("EIFR{}", k / 8) };
    let eicr = |k: usize| format!("EICR{}", (b'A' + (k / 4) as u8) as char);
    let pcr = |g: usize, base: &str| if g / 8 == 0 { base.to_string() } else { format!("{base}{}", g / 8) };
    let mut owned: Vec<(u16, u8, bool)> = Vec::new();
    for j in 0..ints.div_ceil(4) {
        let n = (ints - j * 4).min(4);
        owned.push((r(&eicr(j * 4)), ((1u16 << (n * 2)) - 1) as u8, false));
    }
    for q in 0..ints.div_ceil(8) {
        let n = (ints - q * 8).min(8);
        owned.push((r(&eimsk(q * 8)), ((1u16 << n) - 1) as u8, false));
        owned.push((r(&eifr(q * 8)), 0, true));
    }
    for q in 0..ports.div_ceil(8) {
        let n = (ports - q * 8).min(8);
        owned.push((r(&pcr(q * 8, "PCICR")), ((1u16 << n) - 1) as u8, false));
        owned.push((r(&pcr(q * 8, "PCIFR")), 0, true));
    }
    let ext = ExtInt::new(ExtIntConfig {
        ints: (0..ints)
            .map(|k| IntSpec {
                gpio: gpio(&format!("INT{k}")).unwrap_or_else(|| panic!("{}: no pin carries INT{k}", s.name)), vector: v(&format!("INT{k}")),
                isc_reg: r(&eicr(k)), isc_shift: ((k % 4) * 2) as u8, mask_reg: r(&eimsk(k)), mask_bit: 1 << (k % 8), flag_reg: r(&eifr(k)), flag_bit: 1 << (k % 8),
            })
            .collect(),
        groups: (0..ports)
            .map(|g| PcGroupSpec {
                gpios: (g * 8..g * 8 + 8).map(|x| x as u8).collect(), msk_reg: r(&format!("PCMSK{g}")), vector: v(&format!("PCINT{g}")),
                enable_reg: r(&pcr(g, "PCICR")), enable_bit: 1 << (g % 8), flag_reg: r(&pcr(g, "PCIFR")), flag_bit: 1 << (g % 8),
            })
            .collect(),
        owned,
    });
    let (regs, vecs) = (ext.registers(), ext.vectors());
    add(m, Box::new(ext), regs, &vecs);

    // Timers: numbered like the 2560 (TC0, TC2 8-bit; TC1, TC3-5 16-bit; extras from 6 up).
    let mut psr = Vec::new();
    let mut has_timer2 = false;
    let mut timer_ids = Vec::new();
    for n in 0..32u8 {
        if !has(&format!("TCCR{n}A")) {
            continue;
        }
        timer_ids.push(n);
        let wide = has(&format!("TCCR{n}C"));
        let prr_mask = match n {
            0 => 0x20,
            1 => 0x08,
            2 => 0x40,
            _ => 0,
        } & prr_bits;
        let async2 = n == 2;
        has_timer2 |= async2;
        add_timer(m, TimerConfig {
            name: leak(format!("TC{n}")), id: n, wide,
            tccr_a: r(&format!("TCCR{n}A")),
            tccr_b: r(&format!("TCCR{n}B")),
            foc_reg: if wide { r(&format!("TCCR{n}C")) } else { r(&format!("TCCR{n}B")) },
            tcnt: if wide { r(&format!("TCNT{n}L")) } else { r(&format!("TCNT{n}")) },
            ocr_a: if wide { r(&format!("OCR{n}AL")) } else { r(&format!("OCR{n}A")) },
            ocr_b: if wide { r(&format!("OCR{n}BL")) } else { r(&format!("OCR{n}B")) },
            icr: wide.then(|| r(&format!("ICR{n}L"))),
            tifr: r(&format!("TIFR{n}")), timsk: r(&format!("TIMSK{n}")),
            bits: TimerBits { tov: 0x01, ocfa: 0x02, ocfb: 0x04, icf: if wide { 0x20 } else { 0 } },
            v_ovf: v(&format!("TIMER{n}_OVF")), v_comp_a: v(&format!("TIMER{n}_COMPA")), v_comp_b: v(&format!("TIMER{n}_COMPB")),
            v_capt: wide.then(|| v(&format!("TIMER{n}_CAPT"))),
            oc_a_gpio: gpio(&format!("OC{n}A")).map(usize::from), oc_b_gpio: gpio(&format!("OC{n}B")).map(usize::from),
            icp_gpio: gpio(&format!("ICP{n}")), t_gpio: gpio(&format!("T{n}")),
            clock: if async2 { CS_TIMER2 } else { CS_SYNC }, prescaler_group: if async2 { 2 } else { 1 }, prr_mask,
            // Timer2 keeps running in power-save and extended standby (DS40002061B 10.6).
            sleep_run: if async2 { (1 << SleepKind::PowerSave as u8) | (1 << SleepKind::ExtendedStandby as u8) } else { ALL_SLEEP },
        });
    }
    if !timer_ids.is_empty() {
        if timer_ids.iter().any(|&n| n != 2) {
            psr.push((0x01, 1));
        }
        if has_timer2 {
            psr.push((0x02, 2));
        }
        let g = Gtccr::new(GtccrConfig { addr: r("GTCCR"), tsm: 0x80, psr, strobes: 0, config: 0 });
        let regs = g.registers();
        add(m, Box::new(g), regs, &[]);
    }

    for k in 0..32usize {
        if !has(&format!("UDR{k}")) {
            continue;
        }
        let usart = Usart::new(UsartConfig {
            name: leak(format!("USART{k}")), udr: r(&format!("UDR{k}")), ucsra: r(&format!("UCSR{k}A")), ucsrb: r(&format!("UCSR{k}B")), ucsrc: r(&format!("UCSR{k}C")),
            ubrrl: r(&format!("UBRR{k}L")), ubrrh: r(&format!("UBRR{k}H")), rx_gpio: gpio_u(&format!("RXD{k}")), tx_gpio: gpio_u(&format!("TXD{k}")),
            v_rx: v(&format!("USART{k}_RX")), v_udre: v(&format!("USART{k}_UDRE")), v_tx: v(&format!("USART{k}_TX")), prr_mask: if k == 0 { 0x02 & prr_bits } else { 0 },
        });
        let (regs, vecs) = (usart.registers(), usart.vectors());
        add(m, Box::new(usart), regs, &vecs);
    }

    for k in 0..32usize {
        let (spdr, vname) = (format!("SPDR{}", sx(k)), if k == 0 { "SPI_STC".to_string() } else { format!("SPI{k}_STC") });
        if !has(&spdr) {
            continue;
        }
        let spi = Spi::new(SpiConfig {
            spcr: r(&format!("SPCR{}", sx(k))), spsr: r(&format!("SPSR{}", sx(k))), spdr: r(&spdr),
            ss_gpio: gpio_u(&format!("SS{}", sx(k))), mosi_gpio: gpio_u(&format!("MOSI{}", sx(k))), miso_gpio: gpio_u(&format!("MISO{}", sx(k))), sck_gpio: gpio_u(&format!("SCK{}", sx(k))),
            vector: v(&vname), prr_mask: if k == 0 { 0x04 & prr_bits } else { 0 },
        });
        let regs = spi.registers();
        add(m, Box::new(spi), regs, &[Some(v(&vname))]);
    }

    for k in 0..32usize {
        let (twcr, vname) = (format!("TWCR{}", sx(k)), format!("TWI{}", sx(k)));
        if !has(&twcr) {
            continue;
        }
        let q = |x: &str| r(&format!("{x}{}", sx(k)));
        let twi = Twi::new(TwiConfig { twbr: q("TWBR"), twsr: q("TWSR"), twar: q("TWAR"), twdr: q("TWDR"), twcr: q("TWCR"), twamr: q("TWAMR"), vector: v(&vname), prr_mask: if k == 0 { 0x80 & prr_bits } else { 0 } });
        let regs = twi.registers();
        add(m, Box::new(twi), regs, &[Some(v(&vname))]);
    }

    if has("ACSR") {
        let ac = AnalogComparator::new(AcConfig {
            acsr: r("ACSR"), ain0_gpio: gpio_u("AIN0") as u8, ain1_gpio: gpio_u("AIN1") as u8, vector: v("ANALOG_COMP"), acbg: true, acic: timer_ids.contains(&1),
            acme: has("ADCSRB").then(|| AcmeConfig {
                reg: r("ADCSRB"), bit: 0x40, adcsra: r("ADCSRA"), admux: r("ADMUX"), mux_mask: 0x07,
                channels: (0..8).map(|i| (i < adc_ch).then(|| gpio_u(&format!("ADC{i}")))).collect(),
            }),
        });
        let regs = ac.registers();
        add(m, Box::new(ac), regs, &[Some(v("ANALOG_COMP"))]);
    }

    if has("ADCSRA") {
        let mut inputs: Vec<Option<AdcInput>> = (0..32).map(|i| (i < adc_ch).then(|| AdcInput::Pin(gpio_u(&format!("ADC{i}"))))).collect();
        inputs[30] = Some(AdcInput::Volts(BANDGAP_V));
        inputs[31] = Some(AdcInput::Volts(0.0));
        let tc = |n: u8| timer_ids.contains(&n);
        let adc = Adc::new(AdcConfig {
            adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), adch: Some(r("ADCH")), mux_mask: 0x1f, inputs,
            // REFS1:0 = 00 AREF (tied to VCC here), 01 AVCC, 11 internal 1.1 V.
            ref_mask: 0xc0, ref_extra: 0, refs: vec![Some(AdcRef::Aref(None)), Some(AdcRef::Vcc), None, Some(AdcRef::Volts(BANDGAP_V))],
            adlar: 0x20, adlar_srb: false, admux_mask: 0xff, adcsrb_mask: if has("ACSR") { 0x47 } else { 0x07 }, bin: 0,
            triggers: [
                None,
                has("ACSR").then_some(Trigger::Ac),
                (ints > 0).then_some(Trigger::Int0),
                tc(0).then_some(Trigger::TimerCompA(0)),
                tc(0).then_some(Trigger::TimerOvf(0)),
                tc(1).then_some(Trigger::TimerCompB(1)),
                tc(1).then_some(Trigger::TimerOvf(1)),
                tc(1).then_some(Trigger::TimerCapt(1)),
            ],
            vector: v("ADC"), prr_mask: 0x01 & prr_bits, notify: true,
        });
        let regs = adc.registers();
        add(m, Box::new(adc), regs, &[Some(v("ADC"))]);
    }

    if has("EECR") {
        let ee = Eeprom::new(EepromConfig { eecr: r("EECR"), eedr: r("EEDR"), eearl: r("EEARL"), eearh: Some(r("EEARH")), vector: v("EE_READY") });
        let regs = ee.registers();
        add(m, Box::new(ee), regs, &[Some(v("EE_READY"))]);
    }

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("MCUSR"), vector: v("WDT"), wdce: true });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[Some(v("WDT"))]);

    // Wake-up sources per sleep mode (DS2549 table 10-1 pattern, extended to all instances).
    let mut pd: Vec<String> = (0..ints).map(|k| format!("INT{k}")).chain((0..ports).map(|g| format!("PCINT{g}"))).collect();
    pd.extend(s.vectors.iter().filter(|x| x.name.starts_with("TWI")).map(|x| x.name.clone()));
    pd.push("WDT".into());
    let mut ps = pd.clone();
    ps.extend(["TIMER2_COMPA", "TIMER2_COMPB", "TIMER2_OVF"].map(String::from));
    let mut nr = ps.clone();
    nr.extend(["SPM_READY", "EE_READY", "ADC"].map(String::from));
    let (pd, ps, nr): (Vec<&str>, Vec<&str>, Vec<&str>) = (pd.iter().map(String::as_str).collect(), ps.iter().map(String::as_str).collect(), nr.iter().map(String::as_str).collect());
    set_wake(m, &[
        (SleepKind::AdcNoiseReduction, &nr),
        (SleepKind::PowerDown, &pd),
        (SleepKind::PowerSave, &ps),
        (SleepKind::Standby, &pd),
        (SleepKind::ExtendedStandby, &ps),
    ]);
}
