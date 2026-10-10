//! RCC (reset and clock control), FLASH interface and PWR for the STM32G4 (RM0440 sections 5, 6, 3).
//!
//! RCC: HSI16 / HSE / PLL, SYSCLK switch, AHB and APB prescalers, peripheral clock enable and reset
//! registers. The CPU clock (HCLK) and the APB ratios are published in [`ArmSys`](crate::arm::sys::ArmSys),
//! which also keeps the cycle -> seconds model continuous across changes.
//!
//! Simplifications (documented deviations):
//! * Oscillators and the PLL report "ready" immediately after being enabled (real silicon needs
//!   microseconds to milliseconds); polling loops therefore exit at once.
//! * HSE is an ideal crystal / external clock of `ArmSys::hse_hz` (default 8 MHz, settable through
//!   the external clock input of the UI).
//! * LSI/LSE/HSI48/MCO/CSS/clock-interrupt flags are stored but have no effect; CCIPR kernel clock
//!   selection is ignored (USART/LPUART always run from PCLK).
//! * Disabling a peripheral clock only blocks register access; a running timer or UART keeps going.
//! * FLASH wait states are checked (a warning is logged when LATENCY is too low for HCLK or boost
//!   mode is off above 150 MHz) but not charged to the CPU.

use crate::arm::bus::{Cx, Mmio};
use crate::arm::sys::ClockTree;

use super::{lane_read, lane_write};

const CR: usize = 0x00 >> 2;
const CFGR: usize = 0x08 >> 2;
const PLLCFGR: usize = 0x0c >> 2;
const CIFR: usize = 0x1c >> 2;
const CICR: usize = 0x20 >> 2;

const CR_HSION: u32 = 1 << 8;
const CR_HSIRDY: u32 = 1 << 10;
const CR_HSEON: u32 = 1 << 16;
const CR_HSERDY: u32 = 1 << 17;
const CR_HSEBYP: u32 = 1 << 18;
const CR_PLLON: u32 = 1 << 24;
const CR_PLLRDY: u32 = 1 << 25;
const CR_WRITABLE: u32 = CR_HSION | 1 << 9 | CR_HSEON | CR_HSEBYP | 1 << 19 | CR_PLLON;

pub struct Rcc {
    r: [u32; 40],
    /// SWS: the clock actually driving SYSCLK (1 HSI16, 2 HSE, 3 PLL).
    sws: u32,
    hsi_hz: f64,
}

impl Rcc {
    pub fn new(hsi_hz: f64) -> Self {
        let mut r = Self { r: [0; 40], sws: 1, hsi_hz };
        r.set_reset_values();
        r
    }

    fn set_reset_values(&mut self) {
        self.r = [0; 40];
        self.r[CR] = CR_HSION | CR_HSIRDY;
        self.r[0x04 >> 2] = 0x4000_0000; // ICSCR (trim default; HSICAL is factory programmed)
        self.r[CFGR] = 0x5; // SW = SWS = HSI16
        self.r[PLLCFGR] = 0x0000_1000;
        self.r[0x48 >> 2] = 0x100; // AHB1ENR: FLASHEN
        self.r[0x58 >> 2] = 0x400; // APB1ENR1: RTCAPBEN
        self.r[0x68 >> 2] = 0x0000_1303; // AHB1SMENR
        self.r[0x6c >> 2] = 0x0001_20ff; // AHB2SMENR (reset values per RM0440 6.4)
        self.r[0x94 >> 2] = 0x0c00_0000; // CSR: reset flags
        self.sws = 1;
    }

    fn enr_index(off: usize) -> Option<usize> {
        match off {
            0x48 => Some(0),
            0x4c => Some(1),
            0x50 => Some(2),
            0x58 => Some(3),
            0x5c => Some(4),
            0x60 => Some(5),
            _ => None,
        }
    }

    fn rstr_index(off: usize) -> Option<usize> {
        match off {
            0x28 => Some(0),
            0x2c => Some(1),
            0x30 => Some(2),
            0x38 => Some(3),
            0x3c => Some(4),
            0x40 => Some(5),
            _ => None,
        }
    }

    fn pll_source_hz(&self, cx: &Cx) -> f64 {
        match self.r[PLLCFGR] & 3 {
            2 => self.hsi_hz,
            3 => cx.sys.hse_hz,
            _ => 0.0,
        }
    }

    /// PLL "R" output frequency (the SYSCLK source): `src / M * N / R`.
    fn pll_r_hz(&self, cx: &Cx) -> f64 {
        let c = self.r[PLLCFGR];
        let m = ((c >> 4) & 0xf) + 1;
        let n = (c >> 8) & 0x7f;
        let r = 2 * (((c >> 25) & 3) + 1);
        if n < 8 {
            return 0.0;
        }
        self.pll_source_hz(cx) / m as f64 * n as f64 / r as f64
    }

    fn hse_in_use(&self) -> bool {
        self.sws == 2 || (self.r[CR] & CR_PLLON != 0 && self.r[PLLCFGR] & 3 == 3)
    }

    fn hsi_in_use(&self) -> bool {
        self.sws == 1 || (self.r[CR] & CR_PLLON != 0 && self.r[PLLCFGR] & 3 == 2)
    }

    /// Updates the ready flags and the SYSCLK switch status after any register change, then
    /// publishes the clock tree.
    fn update(&mut self, cx: &mut Cx) {
        let cr = self.r[CR];
        let mut cr = cr & !(CR_HSIRDY | CR_HSERDY | CR_PLLRDY);
        if cr & CR_HSION != 0 {
            cr |= CR_HSIRDY;
        }
        if cr & CR_HSEON != 0 {
            cr |= CR_HSERDY;
        }
        let pll_ok = cr & CR_PLLON != 0 && self.pll_r_hz(cx) > 0.0 && (self.r[PLLCFGR] & 3 != 3 || cr & CR_HSEON != 0) && (self.r[PLLCFGR] & 3 != 2 || cr & CR_HSION != 0);
        if pll_ok {
            cr |= CR_PLLRDY;
        }
        self.r[CR] = cr;
        // The switch completes once the selected source is ready.
        let sw = self.r[CFGR] & 3;
        let ready = match sw {
            1 => cr & CR_HSIRDY != 0,
            2 => cr & CR_HSERDY != 0,
            3 => cr & CR_PLLRDY != 0,
            _ => false,
        };
        if ready {
            self.sws = sw;
        } else if self.sws == 3 && cr & CR_PLLRDY == 0 {
            self.sws = 1; // PLL lost (e.g. HSE disabled): fall back to HSI16
        } else if self.sws == 2 && cr & CR_HSERDY == 0 {
            self.sws = 1;
        }
        self.r[CFGR] = (self.r[CFGR] & !(3 << 2)) | self.sws << 2;
        let sysclk = match self.sws {
            2 => cx.sys.hse_hz,
            3 => self.pll_r_hz(cx),
            _ => self.hsi_hz,
        };
        let hpre = self.r[CFGR] >> 4 & 0xf;
        let hdiv = if hpre < 8 { 1 } else { [2, 4, 8, 16, 64, 128, 256, 512][(hpre - 8) as usize] };
        let ppre = |v: u32| if v < 4 { 1 } else { 1u32 << (v - 3) };
        let hclk = sysclk / hdiv as f64;
        let tree = ClockTree { sysclk_hz: sysclk, hclk_hz: hclk, ppre1: ppre(self.r[CFGR] >> 8 & 7), ppre2: ppre(self.r[CFGR] >> 11 & 7) };
        let cycles = cx.cycles;
        cx.sys.set_clock_tree(tree, cycles);
        self.check_limits(hclk, cx);
        // Peripheral clock gating mirror for the other peripherals.
        for (i, off) in [0x48usize, 0x4c, 0x50, 0x58, 0x5c, 0x60].iter().enumerate() {
            cx.sys.enr[i] = self.r[off >> 2];
        }
    }

    /// Logs the conditions under which real silicon would misbehave at this clock.
    fn check_limits(&self, hclk: f64, cx: &mut Cx) {
        let need = [20e6, 40e6, 60e6, 80e6, 100e6, 120e6, 140e6, 160e6].iter().take_while(|&&f| hclk > f).count() as u8;
        let cycles = cx.cycles;
        if need > cx.sys.flash_latency {
            let (have, mhz) = (cx.sys.flash_latency, hclk / 1e6);
            cx.sys.warn_key(cycles, format!("flash-latency-{need}"), format!("HCLK = {mhz} MHz needs FLASH_ACR.LATENCY >= {need} wait states (currently {have}); real silicon would fetch wrong instructions"));
        }
        if hclk > 150e6 && !cx.sys.boost {
            let mhz = hclk / 1e6;
            cx.sys.warn_key(cycles, "boost-mode", format!("HCLK = {mhz} MHz requires Range 1 boost mode (clear PWR_CR5.R1MODE) and VOS = Range 1"));
        }
    }

    fn write_reg(&mut self, i: usize, v: u32, cx: &mut Cx) {
        let off = i << 2;
        match i {
            CR => {
                let old = self.r[CR];
                let mut new = (old & !CR_WRITABLE) | (v & CR_WRITABLE);
                if old & CR_HSEON != 0 {
                    new = (new & !CR_HSEBYP) | (old & CR_HSEBYP); // HSEBYP only changes while HSE is off
                }
                self.r[CR] = new;
                // Oscillators/PLL in use cannot be switched off.
                if self.hsi_in_use() {
                    self.r[CR] |= CR_HSION;
                }
                if self.hse_in_use() {
                    self.r[CR] |= CR_HSEON;
                }
                if self.sws == 3 {
                    self.r[CR] |= CR_PLLON;
                }
            }
            CFGR => {
                // SWS is read-only; SW = 0 is reserved.
                let n = v & !(3 << 2);
                self.r[CFGR] = (self.r[CFGR] & (3 << 2)) | n;
                if v & 3 == 0 {
                    self.r[CFGR] = (self.r[CFGR] & !3) | self.sws;
                }
            }
            PLLCFGR => {
                if self.r[CR] & (CR_PLLON | CR_PLLRDY) == 0 {
                    self.r[PLLCFGR] = v;
                }
            }
            CIFR => {}
            CICR => self.r[CIFR] &= !v,
            _ => {
                if let Some(k) = Self::enr_index(off) {
                    self.r[i] = v;
                    cx.sys.enr[k] = v;
                } else if let Some(k) = Self::rstr_index(off) {
                    let rising = v & !self.r[i];
                    self.r[i] = v;
                    for b in 0..32u8 {
                        if rising >> b & 1 != 0 {
                            cx.sys.resets.push((k as u8, b));
                            cx.sys.attn = true;
                        }
                    }
                } else if i < self.r.len() {
                    self.r[i] = v;
                }
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
        self.update(cx);
    }

    /// HSE frequency changed (external clock input): recompute the tree.
    fn on_clock_change(&mut self, cx: &mut Cx) {
        self.update(cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let c = &cx.sys.clk;
        let src = ["HSI16", "HSI16", "HSE", "PLL"][self.sws as usize & 3];
        vec![
            ("SYSCLK".into(), format!("{} MHz ({src})", c.sysclk_hz / 1e6)),
            ("HCLK".into(), format!("{} MHz", c.hclk_hz / 1e6)),
            ("PCLK1".into(), format!("{} MHz", c.hclk_hz / c.ppre1 as f64 / 1e6)),
            ("PCLK2".into(), format!("{} MHz", c.hclk_hz / c.ppre2 as f64 / 1e6)),
        ]
    }
}

// -------------------------------------------------------------------------------------------
// FLASH interface
// -------------------------------------------------------------------------------------------

const KEY1: u32 = 0x4567_0123;
const KEY2: u32 = 0xCDEF_89AB;

/// Embedded flash interface registers (ACR wait states / caches, key sequence, status). Flash
/// programming and erase are not modelled: the CR operation bits are accepted and ignored.
pub struct FlashIf {
    acr: u32,
    sr: u32,
    cr: u32,
    keyr_state: u8,
    optkey_state: u8,
}

impl FlashIf {
    pub fn new() -> Self {
        let mut f = Self { acr: 0, sr: 0, cr: 0, keyr_state: 0, optkey_state: 0 };
        f.set_reset_values();
        f
    }

    fn set_reset_values(&mut self) {
        self.acr = 0x0004_0600; // DBG_SWEN, DCEN, ICEN; LATENCY = 0 (STM32G474xx.svd)
        self.sr = 0;
        self.cr = 0xc000_0000; // LOCK and OPTLOCK set
        self.keyr_state = 0;
        self.optkey_state = 0;
    }
}

impl Default for FlashIf {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmio for FlashIf {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        if !cx.sys.clock_on(0, 8) {
            return 0;
        }
        let v = match offset & !3 {
            0x00 => self.acr & !(1 << 11 | 1 << 12), // ICRST / DCRST read as 0
            0x10 => self.sr,
            0x14 => self.cr,
            _ => 0,
        };
        lane_read(v, offset, size)
    }

    fn peek(&mut self, offset: u32, _cx: &mut Cx) -> u32 {
        match offset & !3 {
            0x00 => self.acr & !(1 << 11 | 1 << 12),
            0x10 => self.sr,
            0x14 => self.cr,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if !cx.sys.clock_on(0, 8) {
            return;
        }
        match offset & !3 {
            0x00 => {
                let v = lane_write(self.acr, offset, size, value);
                self.acr = v & 0x0004_7F0F;
                cx.sys.flash_latency = (v & 0xf) as u8;
            }
            0x08 => {
                self.keyr_state = match (self.keyr_state, value) {
                    (0, KEY1) => 1,
                    (1, KEY2) => {
                        self.cr &= !(1 << 31);
                        0
                    }
                    _ => 0,
                };
            }
            0x0c => {
                self.optkey_state = match (self.optkey_state, value) {
                    (0, 0x0819_2A3B) => 1,
                    (1, 0x4C5D_6E7F) => {
                        self.cr &= !(1 << 30);
                        0
                    }
                    _ => 0,
                };
            }
            0x10 => self.sr &= !lane_write(0, offset, size, value),
            0x14 => {
                // LOCK/OPTLOCK are set by writing 1 and cleared only by the key sequence; while LOCK
                // is set the other bits cannot be written. Operations (PG, PER, ...) are not modelled.
                let v = lane_write(self.cr, offset, size, value);
                let locks = (self.cr | v) & (3 << 30);
                self.cr = if self.cr >> 31 == 0 { (v & !(3 << 30)) | locks } else { (self.cr & !(3 << 30)) | locks };
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

/// Power controller: plain registers. VOS/boost settings are stored (`R1MODE` is published to the
/// RCC limit checks); voltage scaling is instantaneous (`PWR_SR2.VOSF` always 0).
pub struct Pwr {
    r: [u32; 0x24],
}

impl Pwr {
    pub fn new() -> Self {
        let mut p = Self { r: [0; 0x24] };
        p.set_reset_values();
        p
    }

    fn set_reset_values(&mut self) {
        self.r = [0; 0x24];
        self.r[0] = 0x0000_0200; // CR1: VOS = Range 1
        self.r[2] = 0x0000_8000; // CR3: EIWUL
        self.r[0x80 >> 2] = 0x0000_0100; // CR5: R1MODE = 1 (boost off)
    }
}

impl Default for Pwr {
    fn default() -> Self {
        Self::new()
    }
}

impl Mmio for Pwr {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        if !cx.sys.clock_on(3, 28) {
            return 0;
        }
        lane_read(self.r.get((offset >> 2) as usize).copied().unwrap_or(0), offset, size)
    }

    fn peek(&mut self, offset: u32, _cx: &mut Cx) -> u32 {
        self.r.get((offset >> 2) as usize).copied().unwrap_or(0)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if !cx.sys.clock_on(3, 28) {
            return;
        }
        let i = (offset >> 2) as usize;
        if i >= self.r.len() || i == 4 || i == 5 {
            return; // SR1 / SR2 are read-only
        }
        self.r[i] = lane_write(self.r[i], offset, size, value);
        if i == 0x80 >> 2 {
            cx.sys.boost = self.r[i] & 0x100 == 0;
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        cx.sys.boost = false;
    }
}
