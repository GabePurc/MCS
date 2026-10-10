//! GPIO port (STM32G4, RM0440 section 8): MODER, OTYPER, OSPEEDR, PUPDR, IDR, ODR, BSRR, LCKR,
//! AFRL/AFRH and BRR driving the electrical pin model in [`ArmSys`](crate::arm::sys::ArmSys).
//!
//! * The port registers are only accessible while the port's AHB2 clock is enabled (RCC_AHB2ENR);
//!   with the clock off reads return 0 and writes are ignored (as on silicon).
//! * Output speed (OSPEEDR) is stored but has no electrical effect.
//! * The LCKR key sequence (write 1/0/1 of LCKK with the same lock mask, then read) is modelled; locked
//!   pins keep MODER/OTYPER/OSPEEDR/PUPDR/AFR bits until the next reset.

use crate::arm::bus::{Cx, Mmio};
use crate::arm::sys::PinCfg;

use super::{lane_read, lane_write};

const MODER: u32 = 0x00;
const OTYPER: u32 = 0x04;
const OSPEEDR: u32 = 0x08;
const PUPDR: u32 = 0x0c;
const IDR: u32 = 0x10;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const LCKR: u32 = 0x1c;
const AFRL: u32 = 0x20;
const AFRH: u32 = 0x24;
const BRR: u32 = 0x28;

pub struct Gpio {
    port: u8,
    /// RCC clock-enable bit in AHB2ENR.
    en_bit: u8,
    moder: u32,
    otyper: u32,
    ospeedr: u32,
    pupdr: u32,
    odr: u32,
    afr: [u32; 2],
    lock_mask: u32,
    lckk: bool,
    /// Lock key sequence progress (0-3) and the mask written with it.
    lock_seq: u8,
    lock_tmp: u32,
}

impl Gpio {
    pub fn new(port: u8) -> Self {
        let mut g = Self { port, en_bit: port, moder: 0, otyper: 0, ospeedr: 0, pupdr: 0, odr: 0, afr: [0; 2], lock_mask: 0, lckk: false, lock_seq: 0, lock_tmp: 0 };
        g.set_reset_values();
        g
    }

    /// Reset values (RM0440 8.4): JTAG/SWD pins start in alternate function, everything else analog.
    fn set_reset_values(&mut self) {
        let (moder, pupdr, ospeedr) = match self.port {
            0 => (0xABFF_FFFF, 0x6400_0000, 0x0C00_0000),
            1 => (0xFFFF_FEBF, 0x0000_0100, 0x0000_00C0),
            _ => (0xFFFF_FFFF, 0, 0),
        };
        self.moder = moder;
        self.pupdr = pupdr;
        self.ospeedr = ospeedr;
        self.otyper = 0;
        self.odr = 0;
        self.afr = [0; 2];
        self.lock_mask = 0;
        self.lckk = false;
        self.lock_seq = 0;
    }

    #[inline]
    fn first_pin(&self) -> usize {
        self.port as usize * 16
    }

    /// Pushes the configuration of the pins in `mask` to the electrical model.
    fn apply(&self, mask: u32, cx: &mut Cx) {
        let base = self.first_pin();
        let cycle = cx.cycles;
        let mut m = mask & 0xffff;
        while m != 0 {
            let b = m.trailing_zeros();
            m &= m - 1;
            let idx = base + b as usize;
            let pu = (self.pupdr >> (2 * b)) & 3;
            let af = (self.afr[(b >> 3) as usize] >> (4 * (b & 7))) & 15;
            cx.sys.pcfg[idx] = PinCfg {
                mode: ((self.moder >> (2 * b)) & 3) as u8,
                open_drain: self.otyper >> b & 1 != 0,
                odr: (self.odr >> b & 1) as u8,
                pullup: pu == 1,
                pulldown: pu == 2,
            };
            cx.sys.select_af(idx, af as u8);
            cx.sys.refresh_pin(idx, cycle);
        }
    }

    /// Keeps the bits of locked pins: `new` for unlocked pins, `old` for locked ones.
    fn locked_merge(&self, old: u32, new: u32, bits_per_pin: u32) -> u32 {
        if self.lock_mask == 0 {
            return new;
        }
        let mut keep = 0u32;
        for b in 0..16 {
            if self.lock_mask >> b & 1 != 0 {
                keep |= (((1u64 << bits_per_pin) - 1) as u32) << (b * bits_per_pin);
            }
        }
        (new & !keep) | (old & keep)
    }

    fn idr(&self, cx: &Cx) -> u32 {
        let base = self.first_pin();
        let mut v = 0;
        for b in 0..16 {
            if (self.moder >> (2 * b)) & 3 != 3 {
                v |= (cx.sys.pins[base + b].level as u32) << b;
            }
        }
        v
    }

    fn reg(&self, off: u32, cx: &Cx) -> u32 {
        match off & !3 {
            MODER => self.moder,
            OTYPER => self.otyper,
            OSPEEDR => self.ospeedr,
            PUPDR => self.pupdr,
            IDR => self.idr(cx),
            ODR => self.odr,
            LCKR => self.lock_mask | (self.lckk as u32) << 16,
            AFRL => self.afr[0],
            AFRH => self.afr[1],
            _ => 0,
        }
    }
}

impl Mmio for Gpio {
    fn read(&mut self, offset: u32, size: u8, cx: &mut Cx) -> u32 {
        if !cx.sys.clock_on(1, self.en_bit) {
            return 0;
        }
        if offset & !3 == LCKR && self.lock_seq == 3 {
            // Third step done, the read completes the key sequence.
            self.lckk = true;
            self.lock_mask = self.lock_tmp;
            self.lock_seq = 0;
        } else if offset & !3 == LCKR {
            self.lock_seq = 0;
        }
        lane_read(self.reg(offset, cx), offset, size)
    }

    fn peek(&mut self, offset: u32, cx: &mut Cx) -> u32 {
        self.reg(offset, cx)
    }

    fn write(&mut self, offset: u32, size: u8, value: u32, cx: &mut Cx) {
        if !cx.sys.clock_on(1, self.en_bit) {
            return;
        }
        let off = offset & !3;
        if off != LCKR {
            self.lock_seq = 0;
        }
        match off {
            MODER => {
                let n = lane_write(self.moder, offset, size, value);
                let n = self.locked_merge(self.moder, n, 2);
                let d = n ^ self.moder;
                self.moder = n;
                self.apply(pair_mask(d), cx);
            }
            OTYPER => {
                let n = lane_write(self.otyper, offset, size, value) & 0xffff;
                let n = self.locked_merge(self.otyper, n, 1);
                let d = n ^ self.otyper;
                self.otyper = n;
                self.apply(d, cx);
            }
            OSPEEDR => {
                let n = lane_write(self.ospeedr, offset, size, value);
                self.ospeedr = self.locked_merge(self.ospeedr, n, 2);
            }
            PUPDR => {
                let n = lane_write(self.pupdr, offset, size, value);
                let n = self.locked_merge(self.pupdr, n, 2);
                let d = n ^ self.pupdr;
                self.pupdr = n;
                self.apply(pair_mask(d), cx);
            }
            ODR => {
                let n = lane_write(self.odr, offset, size, value) & 0xffff;
                let d = n ^ self.odr;
                self.odr = n;
                self.apply(d, cx);
            }
            BSRR => {
                let v = lane_write(0, offset, size, value);
                let n = ((self.odr | (v & 0xffff)) & !(v >> 16)) & 0xffff;
                let d = n ^ self.odr;
                self.odr = n;
                self.apply(d, cx);
            }
            BRR => {
                let v = lane_write(0, offset, size, value) & 0xffff;
                let n = self.odr & !v;
                let d = n ^ self.odr;
                self.odr = n;
                self.apply(d, cx);
            }
            LCKR => {
                let v = lane_write(0, offset, size, value);
                let (key, mask) = (v >> 16 & 1, v & 0xffff);
                if self.lckk {
                    return;
                }
                self.lock_seq = match (self.lock_seq, key) {
                    (0, 1) => {
                        self.lock_tmp = mask;
                        1
                    }
                    (1, 0) if mask == self.lock_tmp => 2,
                    (2, 1) if mask == self.lock_tmp => 3,
                    _ => 0,
                };
            }
            AFRL | AFRH => {
                let i = (off == AFRH) as usize;
                let n = lane_write(self.afr[i], offset, size, value);
                // AFR nibbles of the 8 pins; the pair mask expansion covers 2 bits per pin, so
                // widen the nibble difference to a pin mask by hand.
                let n = self.locked_merge_afr(i, self.afr[i], n);
                let d = n ^ self.afr[i];
                self.afr[i] = n;
                let mut pins = 0u32;
                for b in 0..8 {
                    if d >> (4 * b) & 0xf != 0 {
                        pins |= 1 << (b + 8 * i as u32);
                    }
                }
                self.apply(pins, cx);
            }
            _ => {}
        }
    }

    fn reset(&mut self, cx: &mut Cx) {
        self.set_reset_values();
        self.apply(0xffff, cx);
    }

    fn inspect(&self, cx: &Cx) -> Vec<(String, String)> {
        let base = self.first_pin();
        let mut out = Vec::new();
        for b in 0..16 {
            let mode = (self.moder >> (2 * b)) & 3;
            if mode == 3 {
                continue;
            }
            let p = &cx.sys.pins[base + b];
            let what = match mode {
                0 => format!("input{}", match (self.pupdr >> (2 * b)) & 3 { 1 => " pull-up", 2 => " pull-down", _ => "" }),
                1 => format!("output{}", if self.otyper >> b & 1 != 0 { " open-drain" } else { "" }),
                _ => format!("AF{}", (self.afr[b >> 3] >> (4 * (b & 7))) & 15),
            };
            out.push((format!("P{}{}", (b'A' + self.port) as char, b), format!("{what}, level {}", p.level)));
        }
        out
    }
}

impl Gpio {
    fn locked_merge_afr(&self, i: usize, old: u32, new: u32) -> u32 {
        if self.lock_mask == 0 {
            return new;
        }
        let mut keep = 0u32;
        for b in 0..8 {
            if self.lock_mask >> (b + 8 * i) & 1 != 0 {
                keep |= 0xf << (4 * b);
            }
        }
        (new & !keep) | (old & keep)
    }
}

/// Pin mask (bit per pin) of the 2-bit fields that differ in `d`.
#[inline]
fn pair_mask(d: u32) -> u32 {
    let m = (d | d >> 1) & 0x5555_5555;
    // Compress the even bits into 16 pin bits.
    let mut x = m;
    x = (x | x >> 1) & 0x3333_3333;
    x = (x | x >> 2) & 0x0f0f_0f0f;
    x = (x | x >> 4) & 0x00ff_00ff;
    (x | x >> 8) & 0xffff
}
