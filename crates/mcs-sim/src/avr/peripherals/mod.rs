//! Peripheral models and per-family wiring recipes. Each device spec names a
//! `PeripheralSet`; the recipe instantiates reusable models with that device's register
//! addresses, vectors and pin mapping.

pub mod analog;
pub mod exint;
pub mod port;
pub mod stimulus;
pub mod system;
pub mod timer16;

use mcs_core::avr::device::PeripheralSet;

use super::machine::Machine;
use analog::{AcConfig, Adc, AdcConfig, AnalogComparator};
use exint::{ExtInt, ExtIntConfig};
use port::{Port, PortConfig};
use system::{System, SystemConfig, Watchdog, WatchdogConfig};
use timer16::{Timer16, Timer16Config};

pub fn wire(m: &mut Machine) {
    match m.spec.peripheral_set {
        PeripheralSet::TinyRc => wire_tiny_rc(m),
    }
    // Test-bench signal generators (every device).
    let pins = m.sys.pins.len();
    m.stimulus = Some(m.add_peripheral(Box::new(stimulus::Stimulus::new(pins))));
}

fn add(m: &mut Machine, p: Box<dyn super::machine::Peripheral>, regs: Vec<(u16, u8)>, vectors: &[Option<u8>]) -> u8 {
    let idx = m.add_peripheral(p);
    for (addr, rmw) in regs {
        m.claim_io(addr, idx, rmw);
    }
    for &v in vectors {
        m.claim_irq(v, idx);
    }
    idx
}

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

    let port = Port::new(PortConfig {
        name: "PORTB", pin: r("PINB"), ddr: r("DDRB"), port: r("PORTB"), pue: Some(r("PUEB")), didr: Some(r("DIDR0")),
        gpios: vec![0, 1, 2, 3], reset_gpio: Some(3),
    });
    let mut regs: Vec<(u16, u8)> = port.registers().into_iter().map(|a| (a, 0)).collect();
    regs[0].1 = 0xff; // SBI/CBI on PINx only toggle the addressed bit
    add(m, Box::new(port), regs, &[]);

    let ext = ExtInt::new(ExtIntConfig {
        eicra: r("EICRA"), eimsk: r("EIMSK"), eifr: r("EIFR"), pcicr: r("PCICR"), pcifr: r("PCIFR"), pcmsk: r("PCMSK"),
        int0_gpio: 2, int0_vector: v("INT0").unwrap(), pc_vector: v("PCINT0").unwrap(), pc_gpios: vec![0, 1, 2, 3],
    });
    let regs = ext.registers();
    add(m, Box::new(ext), regs, &[v("INT0"), v("PCINT0")]);

    let timer = Timer16::new(Timer16Config {
        name: "TC0",
        tccr_a: r("TCCR0A"), tccr_b: r("TCCR0B"), tccr_c: r("TCCR0C"), tcnt_l: r("TCNT0L"), tcnt_h: r("TCNT0H"),
        ocr_a_l: r("OCR0AL"), ocr_a_h: r("OCR0AH"), ocr_b_l: r("OCR0BL"), ocr_b_h: r("OCR0BH"), icr_l: r("ICR0L"), icr_h: r("ICR0H"),
        timsk: r("TIMSK0"), tifr: r("TIFR0"), gtccr: r("GTCCR"),
        v_capt: v("TIM0_CAPT").unwrap(), v_ovf: v("TIM0_OVF").unwrap(), v_comp_a: v("TIM0_COMPA").unwrap(), v_comp_b: v("TIM0_COMPB").unwrap(),
        oc_a_gpio: 0, oc_b_gpio: 1, icp_gpio: 1, t_gpio: 2, prr_mask: 0x01,
    });
    let regs = timer.registers();
    add(m, Box::new(timer), regs, &[v("TIM0_CAPT"), v("TIM0_OVF"), v("TIM0_COMPA"), v("TIM0_COMPB")]);

    let ac = AnalogComparator::new(AcConfig { acsr: r("ACSR"), ain0_gpio: 0, ain1_gpio: 1, vector: v("ANA_COMP").unwrap() });
    let regs = ac.registers();
    add(m, Box::new(ac), regs, &[v("ANA_COMP")]);

    if s.has_adc {
        let adc = Adc::new(AdcConfig {
            adcsra: r("ADCSRA"), adcsrb: r("ADCSRB"), admux: r("ADMUX"), adcl: r("ADCL"), channels: vec![0, 1, 2, 3], vector: v("ADC").unwrap(), prr_mask: 0x02,
        });
        let regs = adc.registers();
        add(m, Box::new(adc), regs, &[v("ADC")]);
    }

    let wdt = Watchdog::new(WatchdogConfig { wdtcsr: r("WDTCSR"), rstflr: r("RSTFLR"), vector: v("WDT").unwrap() });
    let regs = wdt.registers();
    add(m, Box::new(wdt), regs, &[v("WDT")]);

    // Wake-up sources per sleep mode (datasheet table 8-1).
    let n = m.cpu.vector_count;
    let mask = |names: &[&str]| {
        let mut a = vec![false; n];
        for name in names {
            if let Some(i) = s.vector(name) {
                a[i as usize] = true;
            }
        }
        a
    };
    let all = vec![true; n];
    let adc_nr = mask(&["INT0", "PCINT0", "ADC", "WDT", "VLM"]);
    let deep = mask(&["INT0", "PCINT0", "WDT"]);
    m.cpu.wake_mask = vec![all, adc_nr, deep.clone(), deep.clone(), deep.clone(), deep.clone(), deep.clone(), deep];
}
