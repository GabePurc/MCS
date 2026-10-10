//! RCC, FLASH interface and PWR of the STM32H743 (RM0433 sections 8, 4, 3; the register layout is the
//! CMSIS header's, reset values come from ST's SVD).
//!
//! RCC: HSI (64 MHz, HSIDIV), CSI (4 MHz), HSI48, HSE, PLL1/2/3 (DIVM, DIVN, DIVP/Q/R, fractional
//! FRACN, VCO / input range checks), CFGR SW/SWS, D1CFGR (D1CPRE, HPRE, D1PPRE), D2CFGR (D2PPRE1/2),
//! D3CFGR (D3PPRE), the xxxRSTR / xxxENR registers of the AHB3, AHB1, AHB2, AHB4, APB3, APB1 L/H, APB2
//! and APB4 buses (plus the CPU1 `RCC_C1_xxxENR` copies, OR-ed with the main enable bits as on the
//! dual-core parts). The CPU clock is `sys_ck / D1CPRE`; AXI / AHB1-4 run at `CPU / HPRE` and APBn at
//! `AHB / D?PPRE`, so every peripheral clock is an integer divisor of the CPU clock and the event
//! scheduler (which counts CPU cycles) stays exact. Timer kernel clocks follow TIMPRE (RM0433
//! table "ratio between clock timer and pclk").
//!
//! PWR: CR3 (LDOEN, SCUEN, BYPASS stored), D3CR.VOS with VOSRDY and CSR1.ACTVOSRDY always set, CR2.BRRDY,
//! USB33RDY; VOS0 is VOS1 plus SYSCFG_PWRCR.ODEN. FLASH: ACR (LATENCY, WRHIGHFREQ), bank 1/2 key
//! sequences and control registers (programming and erase are not modelled).
//!
//! Simplifications (documented deviations):
//! * Oscillators and the PLLs report "ready" immediately after being enabled; voltage scaling is
//!   instantaneous.
//! * HSE is an ideal crystal / external clock of `ArmSys::hse_hz` (default 8 MHz).
//! * LSI/LSE/HSI48/MCO/CSS/clock-interrupt flags are stored but have no effect. The kernel clock
//!   selection registers (D1CCIPR, D2CCIP1R, D2CCIP2R, D3CCIPR) are stored and ignored: USART / UART /
//!   LPUART and the timers always run from their bus clock.
//! * Disabling a peripheral clock only blocks register access; a running timer or UART keeps going.
//! * Operating limits are advisory warnings: PLL input / VCO ranges, maximum CPU / AXI / APB clocks
//!   for the active VOS level (revision V: 480 / 240 / 120 MHz at VOS0, 400 / 200 / 100 at VOS1,
//!   300 / 150 / 75 at VOS2, 200 / 100 / 50 at VOS3) and FLASH_ACR.LATENCY against the AXI clock.
//!   Wait states are not charged to the CPU.

use crate::arm::bus::{Cx, Mmio};
use crate::arm::sys::{ClockTree, NENR};

use super::{lane_read, lane_write};

// RCC register word indices.
const CR: usize = 0x00 >> 2;
const HSICFGR: usize = 0x04 >> 2;
const CRRCR: usize = 0x08 >> 2;
const CSICFGR: usize = 0x0c >> 2;
const CFGR: usize = 0x10 >> 2;
const D1CFGR: usize = 0x18 >> 2;
const D2CFGR: usize = 0x1c >> 2;
const D3CFGR: usize = 0x20 >> 2;
const PLLCKSELR: usize = 0x28 >> 2;
const PLLCFGR: usize = 0x2c >> 2;
const PLL1DIVR: usize = 0x30 >> 2;
const PLL1FRACR: usize = 0x34 >> 2;
const D1CCIPR: usize = 0x4c >> 2;
const D3CCIPR: usize = 0x58 >> 2;
const CIER: usize = 0x60 >> 2;
const CIFR: usize = 0x64 >> 2;
const CICR: usize = 0x68 >> 2;
const BDCR: usize = 0x70 >> 2;
const CSR: usize = 0x74 >> 2;
const RSTR0: usize = 0x7c >> 2;
const GCR: usize = 0xa0 >> 2;
const D3AMR: usize = 0xa8 >> 2;
const LPENR0: usize = 0xfc >> 2;
const LPENR_END: usize = 0x11c >> 2;
const RSR: usize = 0xd0 >> 2;
const ENR0: usize = 0xd4 >> 2;
const C1_ENR0: usize = 0x134 >> 2;
const RCC_WORDS: usize = 0x160 >> 2;

const CR_HSION: u32 = 1 << 0;
const CR_HSIRDY: u32 = 1 << 2;
const CR_HSIDIVF: u32 = 1 << 5;
const CR_CSION: u32 = 1 << 7;
const CR_CSIRDY: u32 = 1 << 8;
const CR_HSI48ON: u32 = 1 << 12;
const CR_HSI48RDY: u32 = 1 << 13;
const CR_D1CKRDY: u32 = 1 << 14;
const CR_D2CKRDY: u32 = 1 << 15;
const CR_HSEON: u32 = 1 << 16;
const CR_HSERDY: u32 = 1 << 17;
const CR_HSEBYP: u32 = 1 << 18;
const CR_PLL1ON: u32 = 1 << 24;
/// Writable bits of RCC_CR: HSION, HSIKERON, HSIDIV, CSION, CSIKERON, HSI48ON, HSEON, HSEBYP, HSECSSON, PLLxON.
const CR_WRITABLE: u32 = 0b11 | 0b11 << 3 | CR_CSION | 1 << 9 | CR_HSI48ON | CR_HSEON | CR_HSEBYP | 1 << 19 | 0x1500_0000;

/// Maximum clocks per voltage scaling level VOS0..VOS3 (DS12110, revision V): CPU, AXI / AHB, APB.
const CPU_MAX: [f64; 4] = [480e6, 400e6, 300e6, 200e6];
const AHB_MAX: [f64; 4] = [240e6, 200e6, 150e6, 100e6];
const APB_MAX: [f64; 4] = [120e6, 100e6, 75e6, 50e6];
/// AXI clock limits of 0, 1, ... 4 flash wait states per VOS level (RM0433 table 17, approximate).
const WS_LIMITS: [[f64; 5]; 4] = [
    [70e6, 140e6, 210e6, 225e6, 240e6],
    [70e6, 140e6, 210e6, 225e6, 225e6],
    [55e6, 110e6, 165e6, 225e6, 225e6],
    [45e6, 90e6, 135e6, 180e6, 225e6],
];

/// Frequency of the reset clock tree (HSI, no prescalers): never reported as over the limit.
const RESET_HZ: f64 = 64e6;

/// HPRE / D1CPRE encoding.
fn ahb_div(v: u32) -> u32 {
    if v & 8 == 0 { 1 } else { [2, 4, 8, 16, 64, 128, 256, 512][(v & 7) as usize] }
}

/// D1PPRE / D2PPREx / D3PPRE encoding.
fn apb_div(v: u32) -> u32 {
    if v & 4 == 0 { 1 } else { 2 << (v & 3) }
}

/// Output of one PLL.
#[derive(Clone, Copy, Default)]
struct PllOut {
    vco: f64,
    input: f64,
    p: f64,
    q: f64,
    r: f64,
}

pub struct Rcc {
    r: [u32; RCC_WORDS],
    /// SWS: the clock actually driving SYSCLK (0 HSI, 1 CSI, 2 HSE, 3 PLL1).
    sws: u32,
    hsi_hz: f64,
    csi_hz: f64,
    pll: [PllOut; 3],
    /// Clock of the AXI / AHB buses (CPU clock / HPRE), for the inspector and the limit checks.
    ahb_hz: f64,
}

impl Rcc {
    pub fn new(hsi_hz: f64, csi_hz: f64) -> Self {
        let mut r = Self { r: [0; RCC_WORDS], sws: 0, hsi_hz, csi_hz, pll: [PllOut::default(); 3], ahb_hz: hsi_hz };
        r.set_reset_values();
        r
    }

    fn set_reset_values(&mut self) {
        self.r = [0; RCC_WORDS];
        self.r[CR] = CR_HSION | 1 << 1 | CR_CSION; // HSION, HSIKERON, CSION (ready flags follow in update())
        self.r[HSICFGR] = 0x4000_0000; // HSITRIM default; HSICAL is factory programmed
        self.r[PLLCKSELR] = 0x0202_0200; // DIVM1-3 = 32
        self.r[PLLCFGR] = 0x01ff_0000; // all PLL outputs enabled
        for n in 0..3 {
            self.r[PLL1DIVR + 2 * n] = 0x0101_0280;
        }
        self.sws = 0;
    }

    fn hsidiv(&self) -> f64 {
        (1 << (self.r[CR] >> 3 & 3)) as f64
    }

    /// Frequency of the PLL input selected by PLLSRC (HSI after HSIDIV, CSI, HSE, none).
    fn pll_source_hz(&self, cx: &Cx) -> f64 {
        match self.r[PLLCKSELR] & 3 {
            0 => self.hsi_hz / self.hsidiv(),
            1 => self.csi_hz,
            2 => cx.sys.hse_hz,
            _ => 0.0,
        }
    }

    fn pll_source_on(&self) -> bool {
        match self.r[PLLCKSELR] & 3 {
            0 => self.r[CR] & CR_HSION != 0,
            1 => self.r[CR] & CR_CSION != 0,
            2 => self.r[CR] & CR_HSEON != 0,
            _ => false,
        }
    }

    /// Output frequencies of PLL `n` (0-2) from the current register values, `None` when the PLL
    /// cannot run (DIVM = 0, no source).
    fn pll_calc(&self, n: usize, cx: &Cx) -> Option<PllOut> {
        let divm = (self.r[PLLCKSELR] >> (4 + 8 * n)) & 0x3f;
        let src = self.pll_source_hz(cx);
        if divm == 0 || src <= 0.0 {
            return None;
        }
        let input = src / divm as f64;
        let d = self.r[PLL1DIVR + 2 * n];
        let mut mult = (d & 0x1ff) as f64 + 1.0;
        if self.r[PLLCFGR] >> (4 * n) & 1 != 0 {
            mult += ((self.r[PLL1FRACR + 2 * n] >> 3) & 0x1fff) as f64 / 8192.0;
        }
        let vco = input * mult;
        let en = |bit: u32| self.r[PLLCFGR] >> (16 + 3 * n as u32 + bit) & 1 != 0;
        let out = |field: u32, bit: u32| if en(bit) { vco / (field + 1) as f64 } else { 0.0 };
        Some(PllOut { vco, input, p: out(d >> 9 & 0x7f, 0), q: out(d >> 16 & 0x7f, 1), r: out(d >> 24 & 0x7f, 2) })
    }

    fn pll_bits(n: usize) -> (u32, u32) {
        (CR_PLL1ON << (2 * n), CR_PLL1ON << (2 * n + 1))
    }

    fn any_pll_on(&self) -> bool {
        (0..3).any(|n| self.r[CR] & Self::pll_bits(n).0 != 0)
    }

    fn hsi_in_use(&self) -> bool {
        self.sws == 0 || (self.r[PLLCKSELR] & 3 == 0 && self.any_pll_on())
    }

    fn csi_in_use(&self) -> bool {
        self.sws == 1 || (self.r[PLLCKSELR] & 3 == 1 && self.any_pll_on())
    }

    fn hse_in_use(&self) -> bool {
        self.sws == 2 || (self.r[PLLCKSELR] & 3 == 2 && self.any_pll_on())
    }

    /// Updates the ready flags and the SYSCLK switch status after any register change, then
    /// publishes the clock tree.
    fn update(&mut self, cx: &mut Cx) {
        let mut cr = self.r[CR] & !(CR_HSIRDY | CR_CSIRDY | CR_HSI48RDY | CR_HSERDY | 0x2a00_0000);
        cr |= CR_HSIDIVF | CR_D1CKRDY | CR_D2CKRDY;
        if cr & CR_HSION != 0 {
            cr |= CR_HSIRDY;
        }
        if cr & CR_CSION != 0 {
            cr |= CR_CSIRDY;
        }
        if cr & CR_HSI48ON != 0 {
            cr |= CR_HSI48RDY;
        }
        if cr & CR_HSEON != 0 {
            cr |= CR_HSERDY;
        }
        self.r[CR] = cr;
        let src_on = self.pll_source_on();
        for n in 0..3 {
            let (on, rdy) = Self::pll_bits(n);
            self.pll[n] = PllOut::default();
            if cr & on != 0 && src_on {
                if let Some(p) = self.pll_calc(n, cx) {
                    self.pll[n] = p;
                    cr |= rdy;
                    self.check_pll(n, p, cx);
                }
            }
        }
        self.r[CR] = cr;
        // The switch completes once the selected source is ready.
        let sw = self.r[CFGR] & 7;
        let ready = match sw {
            0 => cr & CR_HSIRDY != 0,
            1 => cr & CR_CSIRDY != 0,
            2 => cr & CR_HSERDY != 0,
            3 => cr & CR_PLL1ON << 1 != 0 && self.pll[0].p > 0.0,
            _ => false,
        };
        if ready {
            self.sws = sw;
        } else {
            let lost = match self.sws {
                1 => cr & CR_CSIRDY == 0,
                2 => cr & CR_HSERDY == 0,
                3 => cr & CR_PLL1ON << 1 == 0 || self.pll[0].p <= 0.0,
                _ => false,
            };
            if lost {
                self.sws = 0; // source lost (e.g. HSE disabled): fall back to HSI
            }
        }
        self.r[CFGR] = (self.r[CFGR] & !(7 << 3)) | self.sws << 3;
        let sysclk = match self.sws {
            1 => self.csi_hz,
            2 => cx.sys.hse_hz,
            3 => self.pll[0].p,
            _ => self.hsi_hz / self.hsidiv(),
        };
        let (d1, d2, d3) = (self.r[D1CFGR], self.r[D2CFGR], self.r[D3CFGR]);
        let cpu = sysclk / ahb_div(d1 >> 8 & 0xf) as f64;
        let hpre = ahb_div(d1 & 0xf);
        self.ahb_hz = cpu / hpre as f64;
        let (p1, p2, p3, p4) = (apb_div(d2 >> 4 & 7), apb_div(d2 >> 8 & 7), apb_div(d1 >> 4 & 7), apb_div(d3 >> 4 & 7));
        // Timer kernel clock: PCLK x (1 if the APB prescaler is 1, else 2; with TIMPRE x4 from /4 on,
        // x2 for /2), i.e. at most the AHB clock.
        let timpre = self.r[CFGR] >> 15 & 1 != 0;
        let mult = |p: u32| match (p, timpre) {
            (1, _) => 1,
            (2, _) | (_, false) => 2,
            _ => 4,
        };
        let tree = ClockTree {
            sysclk_hz: sysclk,
            hclk_hz: cpu,
            ppre1: hpre * p1,
            ppre2: hpre * p2,
            ppre3: hpre * p3,
            ppre4: hpre * p4,
            tim1: hpre * p1 / mult(p1),
            tim2: hpre * p2 / mult(p2),
        };
        let cycles = cx.cycles;
        cx.sys.set_clock_tree(tree, cycles);
        let ahb = self.ahb_hz;
        self.check_limits(cpu, ahb, [ahb / p1 as f64, ahb / p2 as f64, ahb / p3 as f64, ahb / p4 as f64], cx);
        // Low-speed oscillators and the CPU1 enable registers.
        let (lsi, lse) = (self.r[CSR] & 1 != 0, self.r[BDCR] & 1 != 0);
        self.r[CSR] = (self.r[CSR] & !2) | (lsi as u32) << 1;
        self.r[BDCR] = (self.r[BDCR] & !2) | (lse as u32) << 1;
        for k in 0..NENR {
            cx.sys.enr[k] = self.r[ENR0 + k] | self.r[C1_ENR0 + k];
        }
    }

    /// Input range, VCO range and range-selection checks of a running PLL.
    fn check_pll(&self, n: usize, p: PllOut, cx: &mut Cx) {
        let cycles = cx.cycles;
        let num = n + 1;
        if !(1e6..=16e6).contains(&p.input) {
            let mhz = p.input / 1e6;
            cx.sys.warn_key(cycles, format!("pll{num}-input"), format!("PLL{num} input clock {mhz} MHz is outside 1-16 MHz (check DIVM{num}); real silicon would not lock"));
        } else {
            let want = match p.input {
                x if x < 2e6 => 0,
                x if x < 4e6 => 1,
                x if x < 8e6 => 2,
                _ => 3,
            };
            let rge = self.r[PLLCFGR] >> (2 + 4 * n) & 3;
            if rge != want {
                let mhz = p.input / 1e6;
                cx.sys.warn_key(cycles, format!("pll{num}-rge"), format!("PLL{num}RGE = {rge} does not match the {mhz} MHz PLL input (expected {want}); set PLLCFGR.PLL{num}RGE"));
            }
        }
        let medium = self.r[PLLCFGR] >> (1 + 4 * n) & 1 != 0;
        let (lo, hi, name) = if medium { (150e6, 420e6, "medium (150-420 MHz)") } else { (192e6, 960e6, "wide (192-960 MHz)") };
        if p.vco < lo || p.vco > hi {
            let mhz = p.vco / 1e6;
            cx.sys.warn_key(cycles, format!("pll{num}-vco"), format!("PLL{num} VCO at {mhz} MHz is outside the {name} range selected by PLL{num}VCOSEL"));
        }
    }

    /// Logs the conditions under which real silicon would misbehave at this clock.
    fn check_limits(&self, cpu: f64, ahb: f64, pclk: [f64; 4], cx: &mut Cx) {
        let lvl = cx.sys.vos_level() as usize;
        let cycles = cx.cycles;
        let vos = ["VOS0", "VOS1", "VOS2", "VOS3"][lvl];
        // Clocks up to the 64 MHz reset clock never warn: the chip starts there whatever the scaling.
        if cpu > CPU_MAX[lvl] && cpu > RESET_HZ {
            let mhz = cpu / 1e6;
            let max = CPU_MAX[lvl] / 1e6;
            cx.sys.warn_key(cycles, format!("cpu-vos-{lvl}"), format!("CPU clock {mhz} MHz exceeds the {max} MHz allowed at {vos}; raise the voltage scaling (PWR_D3CR.VOS, SYSCFG_PWRCR.ODEN) first"));
        }
        if ahb > AHB_MAX[lvl] && ahb > RESET_HZ {
            let mhz = ahb / 1e6;
            let max = AHB_MAX[lvl] / 1e6;
            cx.sys.warn_key(cycles, format!("ahb-vos-{lvl}"), format!("AXI / AHB clock {mhz} MHz exceeds the {max} MHz allowed at {vos} (check HPRE)"));
        }
        for (i, &f) in pclk.iter().enumerate() {
            if f > APB_MAX[lvl] && f > RESET_HZ {
                let (mhz, max, bus) = (f / 1e6, APB_MAX[lvl] / 1e6, i + 1);
                cx.sys.warn_key(cycles, format!("apb{bus}-vos-{lvl}"), format!("APB{bus} clock {mhz} MHz exceeds the {max} MHz allowed at {vos} (check the APB prescalers)"));
            }
        }
        let need = WS_LIMITS[lvl].iter().take_while(|&&f| ahb > f).count() as u8;
        if need > cx.sys.flash_latency {
            let (have, mhz) = (cx.sys.flash_latency, ahb / 1e6);
            cx.sys.warn_key(cycles, format!("flash-latency-{need}"), format!("AXI clock {mhz} MHz needs FLASH_ACR.LATENCY >= {need} wait states at {vos} (currently {have}); real silicon would fetch wrong instructions"));
        }
    }

    /// Writable-bit mask of the register at word index `i`; `None` for read-only / reserved words.
    fn write_mask(&self, i: usize) -> Option<u32> {
        let on = |n: usize| self.r[CR] & Self::pll_bits(n).0 != 0;
        let divm_locked: u32 = (0..3).filter(|&n| on(n)).map(|n| 0x3f << (4 + 8 * n)).fold(0, |a, b| a | b);
        Some(match i {
            HSICFGR => 0x7f00_0000,
            CSICFGR => 0x3f00_0000,
            CRRCR => 0, // HSI48 calibration, read-only
            D1CFGR => 0xf7f,
            D2CFGR => 0x770,
            D3CFGR => 0x70,
            PLLCKSELR => {
                let src_locked = if self.any_pll_on() { 3 } else { 0 };
                0x03f3_f3f3 & !divm_locked & !src_locked
            }
            PLLCFGR => 0x01ff_0fff,
            x if (PLL1DIVR..=PLL1DIVR + 5).contains(&x) => {
                let n = (x - PLL1DIVR) / 2;
                if (x - PLL1DIVR).is_multiple_of(2) {
                    if on(n) { 0 } else { 0x7f7f_ffff } // DIVx only change while the PLL is off
                } else {
                    0xfff8 // FRACN
                }
            }
            x if (D1CCIPR..=D3CCIPR).contains(&x) => u32::MAX, // kernel clock selection: stored
            CIER | BDCR | CSR | GCR | D3AMR => u32::MAX,
            x if (RSTR0..RSTR0 + 9).contains(&x) => u32::MAX,
            x if (ENR0..ENR0 + 9).contains(&x) => u32::MAX,
            x if (LPENR0..=LPENR_END).contains(&x) => u32::MAX,
            x if (C1_ENR0..C1_ENR0 + 9).contains(&x) => u32::MAX,
            _ => return None,
        })
    }

    fn write_reg(&mut self, i: usize, v: u32, cx: &mut Cx) {
        match i {
            CR => {
                let old = self.r[CR];
                let mut new = (old & !CR_WRITABLE) | (v & CR_WRITABLE);
                if old & CR_HSEON != 0 {
                    new = (new & !CR_HSEBYP) | (old & CR_HSEBYP); // HSEBYP only changes while HSE is off
                }
                new |= old & 1 << 19; // HSECSSON is set by software, cleared by a CSS event only
                self.r[CR] = new;
                // Oscillators in use cannot be switched off.
                if self.hsi_in_use() {
                    self.r[CR] |= CR_HSION;
                }
                if self.csi_in_use() {
                    self.r[CR] |= CR_CSION;
                }
                if self.hse_in_use() {
                    self.r[CR] |= CR_HSEON;
                }
                if self.sws == 3 {
                    self.r[CR] |= CR_PLL1ON;
                }
            }
            CFGR => {
                // SWS is read-only; SW values above 3 are reserved and ignored.
                let keep_sw = v & 7 > 3;
                let mut n = (v & !(7 << 3)) & 0xffff_ffc7;
                if keep_sw {
                    n = (n & !7) | (self.r[CFGR] & 7);
                }
                self.r[CFGR] = (self.r[CFGR] & (7 << 3)) | n;
            }
            CIFR => {}
            CICR => self.r[CIFR] &= !v,
            RSR => {
                // RMVF clears the reset flags.
                if v & 1 << 16 != 0 {
                    self.r[RSR] = 0;
                }
            }
            _ => {
                let Some(mask) = self.write_mask(i) else { return };
                let new = (self.r[i] & !mask) | (v & mask);
                if (RSTR0..RSTR0 + 9).contains(&i) {
                    let rising = new & !self.r[i];
                    for b in 0..32u8 {
                        if rising >> b & 1 != 0 {
                            cx.sys.resets.push(((i - RSTR0) as u8, b));
                            cx.sys.attn = true;
                        }
                    }
                }
                self.r[i] = new;
            }
        }
        self.update(cx);
    }
}

impl Mmio for Rcc {
    fn read(&mut self, offset: u32, size: u8, _cx: &mut Cx) -> u32 {
        let i = (offset >> 2) as usize;
        lane_read(self.r.get(i).copied().unwrap_or(0), offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        let i = (offset >> 2) as usize;
        if i >= self.r.len() {
            return;
        }
        let v = lane_write(self.r[i], offset, size, value);
        self.write_reg(i, v, cx);
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        cx.sys.flash_latency = 7; // FLASH_ACR resets to LATENCY = 7 (the FLASH reset follows)
        self.update(cx);
    }

    /// HSE frequency changed (external clock input): recompute the tree.
    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.update(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let c = &cx.sys.clk;
        let src = ["HSI", "CSI", "HSE", "PLL1"][self.sws as usize & 3];
        let mhz = |hz: f64| format!("{} MHz", hz / 1e6);
        let mut v = vec![
            ("SYSCLK".into(), format!("{} ({src})", mhz(c.sysclk_hz))),
            ("CPU clock".into(), mhz(c.hclk_hz)),
            ("AXI / AHB".into(), mhz(self.ahb_hz)),
        ];
        for (i, div) in [c.ppre1, c.ppre2, c.ppre3, c.ppre4].into_iter().enumerate() {
            v.push((format!("PCLK{}", i + 1), mhz(c.hclk_hz / div as f64)));
        }
        for (n, p) in self.pll.iter().enumerate() {
            if p.vco > 0.0 {
                v.push((format!("PLL{}", n + 1), format!("VCO {}, P {}, Q {}, R {}", mhz(p.vco), mhz(p.p), mhz(p.q), mhz(p.r))));
            }
        }
        v.push(("VOS".into(), format!("VOS{}", cx.sys.vos_level())));
        v
    }
}

// -------------------------------------------------------------------------------------------
// FLASH interface
// -------------------------------------------------------------------------------------------

const KEY1: u32 = 0x4567_0123;
const KEY2: u32 = 0xCDEF_89AB;
const OPTKEY1: u32 = 0x0819_2A3B;
const OPTKEY2: u32 = 0x4C5D_6E7F;

/// Embedded flash interface (RM0433 section 4.9): ACR wait states, the key sequences of the two banks
/// and the option bytes, and the control / status registers. Programming, erase and option byte
/// changes are not modelled: the CR operation bits are accepted and ignored (START self-clears).
pub struct FlashIf {
    acr: u32,
    keyr: [u8; 2],
    cr: [u32; 2],
    sr: [u32; 2],
    optkey: u8,
    optcr: u32,
}

impl FlashIf {
    pub fn new() -> Self {
        let mut f = Self { acr: 0, keyr: [0; 2], cr: [0; 2], sr: [0; 2], optkey: 0, optcr: 0 };
        f.set_reset_values();
        f
    }

    fn set_reset_values(&mut self) {
        self.acr = 0x37; // LATENCY = 7, WRHIGHFREQ = 3
        self.keyr = [0; 2];
        self.cr = [0x31; 2]; // LOCK, PSIZE = 3
        self.sr = [0; 2];
        self.optkey = 0;
        self.optcr = 1; // OPTLOCK
    }

    fn read_word(&self, off: u32) -> u32 {
        match off & !3 {
            0x00 | 0x100 => self.acr,
            0x0c => self.cr[0],
            0x10 => self.sr[0],
            0x18 => self.optcr,
            0x10c => self.cr[1],
            0x110 => self.sr[1],
            _ => 0,
        }
    }
}

impl Default for FlashIf {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmio for FlashIf {
    fn read(&mut self, offset: u32, size: u8, _cx: &mut Cx) -> u32 {
        lane_read(self.read_word(offset), offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        let off = offset & !3;
        match off {
            0x00 | 0x100 => {
                let v = lane_write(self.acr, offset, size, value);
                self.acr = v & 0x3f;
                cx.sys.flash_latency = (v & 0xf) as u8;
            }
            0x04 | 0x104 => {
                let b = (off >> 8) as usize;
                self.keyr[b] = match (self.keyr[b], value) {
                    (0, KEY1) => 1,
                    (1, KEY2) => {
                        self.cr[b] &= !1;
                        0
                    }
                    _ => 0,
                };
            }
            0x08 => {
                self.optkey = match (self.optkey, value) {
                    (0, OPTKEY1) => 1,
                    (1, OPTKEY2) => {
                        self.optcr &= !1;
                        0
                    }
                    _ => 0,
                };
            }
            0x0c | 0x10c => {
                // LOCK is set by writing 1 and cleared only by the key sequence; while LOCK is set the
                // other bits cannot be written. START (bit 7) self-clears: operations are not modelled.
                let b = (off >> 8) as usize;
                let v = lane_write(self.cr[b], offset, size, value);
                let lock = (self.cr[b] | v) & 1;
                self.cr[b] = if self.cr[b] & 1 != 0 { self.cr[b] | lock } else { (v & !(1 << 7) & !1) | lock };
            }
            0x14 | 0x114 => {
                // CCR: write 1 to clear status flags.
                let b = (off >> 8) as usize;
                self.sr[b] &= !lane_write(0, offset, size, value);
            }
            0x18 => {
                let v = lane_write(self.optcr, offset, size, value);
                // OPTLOCK: set by writing 1, cleared by the key sequence; OPTSTART self-clears.
                self.optcr = if self.optcr & 1 != 0 { self.optcr | (v & 1) } else { v & !(1 << 1) };
            }
            _ => {}
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        cx.sys.flash_latency = (self.acr & 0xf) as u8;
    }
}

// -------------------------------------------------------------------------------------------
// PWR
// -------------------------------------------------------------------------------------------

const PWR_WORDS: usize = 0x30 >> 2;
const CR2: usize = 0x08 >> 2;
const CR3: usize = 0x0c >> 2;
const D3CR: usize = 0x18 >> 2;
const CSR1: usize = 0x04 >> 2;

/// Power controller (RM0433 section 6): registers are stored; D3CR.VOS is published to the machine
/// ([`ArmSys::vos_field`](crate::arm::sys::ArmSys)) and combined with SYSCFG_PWRCR.ODEN for the RCC
/// limit checks. Voltage scaling and the supply configuration (LDO, bypass) settle instantly:
/// CSR1.ACTVOSRDY and D3CR.VOSRDY always read 1, CR2.BRRDY follows BREN, CR3.USB33RDY follows USB33DEN.
pub struct Pwr {
    r: [u32; PWR_WORDS],
}

impl Pwr {
    pub fn new() -> Self {
        let mut p = Self { r: [0; PWR_WORDS] };
        p.set_reset_values();
        p
    }

    fn set_reset_values(&mut self) {
        self.r = [0; PWR_WORDS];
        self.r[0] = 0xf000_c000; // CR1
        self.r[CR3] = 0x0000_0006; // LDOEN, SCUEN
        self.r[D3CR] = 0x0000_4000; // VOS3
    }

    fn word(&self, i: usize) -> u32 {
        let v = self.r.get(i).copied().unwrap_or(0);
        match i {
            CSR1 => 1 << 13 | (self.r[D3CR] & 0xc000), // ACTVOSRDY, ACTVOS follows VOS
            CR2 => (v & !(1 << 16)) | (v & 1) << 16,
            CR3 => (v & !(1 << 26)) | (v >> 24 & 1) << 26,
            D3CR => v | 1 << 13, // VOSRDY
            _ => v,
        }
    }
}

impl Default for Pwr {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmio for Pwr {
    fn read(&mut self, offset: u32, size: u8, _cx: &mut Cx) -> u32 {
        lane_read(self.word((offset >> 2) as usize), offset, size)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        let i = (offset >> 2) as usize;
        if i >= PWR_WORDS || i == CSR1 {
            return; // CSR1 is read-only
        }
        let old = self.r[i];
        let mut v = lane_write(old, offset, size, value);
        match i {
            CR2 => v &= 0x11, // BREN, MONEN; BRRDY is derived
            CR3 => v &= 0x0300_0307, // BYPASS, LDOEN, SCUEN, VBE, VBRS, USB33DEN, USBREGEN
            D3CR => v &= 0xc000,
            _ => {}
        }
        self.r[i] = v;
        if i == D3CR {
            cx.sys.vos_field = (v >> 14 & 3) as u8;
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        cx.sys.vos_field = 1;
    }
}
