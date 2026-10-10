//! Unit tests of the FP arithmetic: the exact soft-float against native IEEE arithmetic, the
//! native fast paths against the soft-float (values and flags), directed rounding against an
//! exact f64 reference, conversions against Rust's saturating casts, and hand-derived flag cases.

use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// A mix of special values, random patterns and numbers near one another.
    fn f32(&mut self) -> u32 {
        let r = self.next();
        match r % 8 {
            0 => [0, 0x8000_0000, 1, 0x8000_0001, 0x007f_ffff, 0x0080_0000, 0x7f7f_ffff, 0xff7f_ffff, 0x7f80_0000, 0xff80_0000, 0x3f80_0000, 0xbf80_0000][(r >> 8) as usize % 12],
            1 => (r >> 16) as u32 & 0x807f_ffff, // denormals and zeros
            2 => ((r >> 8) as u32 & 0x8000_0000) | (0x70 + (r >> 40) as u32 % 24) << 23 | (r >> 20) as u32 & 0x7f_ffff,
            _ => (r >> 16) as u32,
        }
    }
    fn f64(&mut self) -> u64 {
        let r = self.next();
        match r % 8 {
            0 => [0, 1 << 63, 1, (1 << 63) | 1, 0x000f_ffff_ffff_ffff, 0x0010_0000_0000_0000, 0x7fef_ffff_ffff_ffff, 0x7ff0_0000_0000_0000, 0xfff0_0000_0000_0000, 0x3ff0_0000_0000_0000][(r >> 8) as usize % 10],
            1 => self.next() & 0x800f_ffff_ffff_ffff,
            2 => (self.next() & (1 << 63)) | (0x3f0 + (r >> 40) % 24) << 52 | self.next() & 0xf_ffff_ffff_ffff,
            _ => self.next(),
        }
    }
}

fn same32(a: u32, b: f32) -> bool {
    let a = f32::from_bits(a);
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}
fn same64(a: u64, b: f64) -> bool {
    let a = f64::from_bits(a);
    (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
}

#[test]
fn soft_float_matches_native_f32_in_round_to_nearest() {
    let mut g = Rng(0x1234_5678_9abc_def1);
    for _ in 0..400_000 {
        let (a, b, c) = (g.f32(), g.f32(), g.f32());
        let (x, y, z) = (f32::from_bits(a), f32::from_bits(b), f32::from_bits(c));
        let mut fp = 0;
        assert!(same32(add_soft(F32, a as u64, b as u64, false, &mut fp) as u32, x + y), "add {a:#x} {b:#x}");
        assert!(same32(add_soft(F32, a as u64, b as u64, true, &mut fp) as u32, x - y), "sub {a:#x} {b:#x}");
        assert!(same32(mul_soft(F32, a as u64, b as u64, &mut fp) as u32, x * y), "mul {a:#x} {b:#x}");
        assert!(same32(div_soft(F32, a as u64, b as u64, &mut fp) as u32, x / y), "div {a:#x} {b:#x}");
        assert!(same32(sqrt_soft(F32, a as u64, &mut fp) as u32, x.sqrt()), "sqrt {a:#x}");
        assert!(same32(fma_soft(F32, c as u64, a as u64, b as u64, &mut fp) as u32, x.mul_add(y, z)), "fma {a:#x} {b:#x} {c:#x}");
    }
}

#[test]
fn soft_float_matches_native_f64_in_round_to_nearest() {
    let mut g = Rng(0x0fed_cba9_8765_4321);
    for _ in 0..400_000 {
        let (a, b, c) = (g.f64(), g.f64(), g.f64());
        let (x, y, z) = (f64::from_bits(a), f64::from_bits(b), f64::from_bits(c));
        let mut fp = 0;
        assert!(same64(add_soft(F64, a, b, false, &mut fp), x + y), "add {a:#x} {b:#x}");
        assert!(same64(add_soft(F64, a, b, true, &mut fp), x - y), "sub {a:#x} {b:#x}");
        assert!(same64(mul_soft(F64, a, b, &mut fp), x * y), "mul {a:#x} {b:#x}");
        assert!(same64(div_soft(F64, a, b, &mut fp), x / y), "div {a:#x} {b:#x}");
        assert!(same64(sqrt_soft(F64, a, &mut fp), x.sqrt()), "sqrt {a:#x}");
        assert!(same64(fma_soft(F64, c, a, b, &mut fp), x.mul_add(y, z)), "fma {a:#x} {b:#x} {c:#x}");
    }
}

type Op32 = fn(u32, u32, &mut u32) -> u32;
type Soft32 = fn(u64, u64, &mut u32) -> u64;

#[test]
fn fast_paths_agree_with_soft_float_including_flags() {
    let mut g = Rng(0xdead_beef_cafe_f00d);
    for _ in 0..400_000 {
        let (a, b) = (g.f32(), g.f32());
        let ops: [(&str, Op32, Soft32); 4] = [
            ("add", |a, b, f| add32(a, b, false, f), |a, b, f| add_soft(F32, a, b, false, f)),
            ("sub", |a, b, f| add32(a, b, true, f), |a, b, f| add_soft(F32, a, b, true, f)),
            ("mul", mul32, |a, b, f| mul_soft(F32, a, b, f)),
            ("div", div32, |a, b, f| div_soft(F32, a, b, f)),
        ];
        for (name, fast, soft) in ops {
            let (mut f1, mut f2) = (0u32, 0u32);
            let r1 = fast(a, b, &mut f1);
            let r2 = soft(a as u64, b as u64, &mut f2) as u32;
            assert!(same32(r1, f32::from_bits(r2)), "{name} {a:#x} {b:#x}: {r1:#x} vs {r2:#x}");
            assert_eq!(f1, f2, "{name} {a:#x} {b:#x} flags");
        }
        let (mut f1, mut f2) = (0u32, 0u32);
        assert!(same32(sqrt32(a, &mut f1), f32::from_bits(sqrt_soft(F32, a as u64, &mut f2) as u32)));
        assert_eq!(f1, f2, "sqrt {a:#x} flags");
        let (a, b) = (g.f64(), g.f64());
        let ops: [(&str, Soft32, Soft32); 4] = [
            ("add", |a, b, f| add64(a, b, false, f), |a, b, f| add_soft(F64, a, b, false, f)),
            ("sub", |a, b, f| add64(a, b, true, f), |a, b, f| add_soft(F64, a, b, true, f)),
            ("mul", mul64, |a, b, f| mul_soft(F64, a, b, f)),
            ("div", div64, |a, b, f| div_soft(F64, a, b, f)),
        ];
        for (name, fast, soft) in ops {
            let (mut f1, mut f2) = (0u32, 0u32);
            let r1 = fast(a, b, &mut f1);
            let r2 = soft(a, b, &mut f2);
            assert!(same64(r1, f64::from_bits(r2)), "{name} {a:#x} {b:#x}");
            assert_eq!(f1, f2, "{name} {a:#x} {b:#x} flags");
        }
        let (mut f1, mut f2) = (0u32, 0u32);
        assert!(same64(sqrt64(a, &mut f1), f64::from_bits(sqrt_soft(F64, a, &mut f2))));
        assert_eq!(f1, f2, "sqrt {a:#x} flags");
    }
}

fn next_up(r: f32) -> f32 {
    if r == 0.0 {
        f32::from_bits(1)
    } else if r > 0.0 {
        f32::from_bits(r.to_bits() + 1)
    } else {
        f32::from_bits(r.to_bits() - 1)
    }
}

/// Reference directed rounding of an exact f64 value to f32.
fn ref_round(x: f64, rm: Round) -> f32 {
    let r = x as f32;
    let rf = r as f64;
    match rm {
        Round::Nearest => r,
        Round::Zero => {
            if rf.abs() > x.abs() {
                f32::from_bits(r.to_bits() - 1)
            } else {
                r
            }
        }
        Round::PlusInf => {
            if rf < x {
                next_up(r)
            } else {
                r
            }
        }
        Round::MinusInf => {
            if rf > x {
                -next_up(-r)
            } else {
                r
            }
        }
        Round::Away => unreachable!(),
    }
}

#[test]
fn directed_rounding_of_f32_products_sums_and_fma() {
    let mut g = Rng(0x5555_aaaa_1357_9bdf);
    for _ in 0..200_000 {
        // Moderate exponents keep the f64 reference exact.
        let a = (g.f32() & 0x807f_ffff) | (0x60 + (g.next() % 40) as u32) << 23;
        let b = (g.f32() & 0x807f_ffff) | (0x60 + (g.next() % 40) as u32) << 23;
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        for (rmode, rm) in [(0u32, Round::Nearest), (1, Round::PlusInf), (2, Round::MinusInf), (3, Round::Zero)] {
            let mut fp = rmode << 22;
            let got = f32::from_bits(mul_soft(F32, a as u64, b as u64, &mut fp) as u32);
            assert_eq!(got.to_bits(), ref_round(x as f64 * y as f64, rm).to_bits(), "mul {a:#x} {b:#x} mode {rmode}");
            let mut fp = rmode << 22;
            let got = f32::from_bits(add_soft(F32, a as u64, b as u64, false, &mut fp) as u32);
            let exact = x as f64 + y as f64;
            if exact != 0.0 {
                assert_eq!(got.to_bits(), ref_round(exact, rm).to_bits(), "add {a:#x} {b:#x} mode {rmode}");
            }
            let mut fp = rmode << 22;
            let got = f32::from_bits(fma_soft(F32, x.to_bits() as u64, a as u64, b as u64, &mut fp) as u32);
            let prod = x as f64 * y as f64;
            let sum = prod + x as f64;
            // Compare only when the f64 sum is exact (true when the exponents are close).
            if (sum - prod) == x as f64 && (sum - x as f64) == prod && sum != 0.0 {
                assert_eq!(got.to_bits(), ref_round(sum, rm).to_bits(), "fma {a:#x} {b:#x} mode {rmode}");
            }
        }
    }
}

#[test]
fn exception_flags() {
    let one = 1.0f32.to_bits() as u64;
    let three = 3.0f32.to_bits() as u64;
    let mut fp = 0;
    div_soft(F32, one, three, &mut fp);
    assert_eq!(fp, IXC);
    let mut fp = 0;
    assert_eq!(div_soft(F32, one, 0, &mut fp), 0x7f80_0000);
    assert_eq!(fp, DZC);
    let mut fp = 0;
    assert_eq!(div_soft(F32, 0, 0, &mut fp), 0x7fc0_0000);
    assert_eq!(fp, IOC);
    let mut fp = 0;
    assert_eq!(sqrt_soft(F32, (-1.0f32).to_bits() as u64, &mut fp), 0x7fc0_0000);
    assert_eq!(fp, IOC);
    let mut fp = 0;
    assert_eq!(mul_soft(F32, f32::MAX.to_bits() as u64, 2.0f32.to_bits() as u64, &mut fp), 0x7f80_0000);
    assert_eq!(fp, OFC | IXC);
    // Round toward zero overflows to the largest finite number.
    let mut fp = 3 << 22;
    assert_eq!(mul_soft(F32, f32::MAX.to_bits() as u64, 2.0f32.to_bits() as u64, &mut fp) as u32, f32::MAX.to_bits());
    // Tiny inexact result: underflow; tiny exact result: no underflow.
    let mut fp = 0;
    mul_soft(F32, 0x0080_0001, 0.5f32.to_bits() as u64, &mut fp);
    assert_eq!(fp, UFC | IXC);
    let mut fp = 0;
    mul_soft(F32, 0x0080_0000, 0.5f32.to_bits() as u64, &mut fp);
    assert_eq!(fp, 0);
    // Flush to zero: denormal inputs set IDC, tiny results set UFC.
    let mut fp = FPSCR_FZ;
    assert_eq!(add_soft(F32, 1, one, false, &mut fp), one);
    assert_eq!(fp, FPSCR_FZ | IDC);
    let mut fp = FPSCR_FZ;
    assert_eq!(mul_soft(F32, 0x0080_0000, 0.5f32.to_bits() as u64, &mut fp), 0);
    assert_eq!(fp, FPSCR_FZ | UFC);
    // Signalling NaN: quieted, IOC; default NaN mode replaces the payload.
    let mut fp = 0;
    assert_eq!(add_soft(F32, 0x7f80_0001, one, false, &mut fp), 0x7fc0_0001);
    assert_eq!(fp, IOC);
    let mut fp = FPSCR_DN;
    assert_eq!(add_soft(F32, 0x7fc0_1234, one, false, &mut fp), 0x7fc0_0000);
    assert_eq!(fp, FPSCR_DN);
    // inf - inf
    let mut fp = 0;
    assert_eq!(add_soft(F32, 0x7f80_0000, 0x7f80_0000, true, &mut fp), 0x7fc0_0000);
    assert_eq!(fp, IOC);
    // x + (-x) = +0, or -0 when rounding toward minus infinity.
    let mut fp = 0;
    assert_eq!(add_soft(F32, one, (-1.0f32).to_bits() as u64, false, &mut fp), 0);
    let mut fp = 2 << 22;
    assert_eq!(add_soft(F32, one, (-1.0f32).to_bits() as u64, false, &mut fp), 0x8000_0000);
    // FMA: 0 * inf + QNaN addend gives the default NaN with IOC.
    let mut fp = 0;
    assert_eq!(fma_soft(F32, 0x7fc0_0005, 0, 0x7f80_0000, &mut fp), 0x7fc0_0000);
    assert_eq!(fp, IOC);
}

#[test]
fn float_to_integer_conversions_match_rust_casts() {
    let mut g = Rng(0x2468_ace0_1357_9bdf);
    for _ in 0..200_000 {
        let a = g.f32();
        let x = f32::from_bits(a);
        let mut fp = 0;
        assert_eq!(to_int(F32, a as u64, true, 32, 0, Round::Zero, &mut fp) as i32, x as i32, "s32 {a:#x}");
        let sat = fp & IOC != 0;
        assert_eq!(sat, x.is_nan() || x >= 2147483648.0 || x < -2147483648.0 || x.is_infinite(), "ioc {a:#x}");
        let mut fp = 0;
        assert_eq!(to_int(F32, a as u64, false, 32, 0, Round::Zero, &mut fp), x as u32, "u32 {a:#x}");
        let mut fp = 0;
        let got = to_int(F32, a as u64, true, 32, 0, Round::Nearest, &mut fp) as i32;
        assert_eq!(got, x.round_ties_even() as i32, "rne {a:#x}");
        let d = g.f64();
        let y = f64::from_bits(d);
        let mut fp = 0;
        assert_eq!(to_int(F64, d, true, 32, 0, Round::Zero, &mut fp) as i32, y as i32);
        assert_eq!(to_int(F64, d, false, 32, 0, Round::MinusInf, &mut fp), y.floor() as u32);
        assert_eq!(to_int(F64, d, true, 32, 0, Round::PlusInf, &mut fp) as i32, y.ceil() as i32);
        assert_eq!(to_int(F64, d, true, 32, 0, Round::Away, &mut fp) as i32, y.round() as i32);
    }
    // Fixed point with 16-bit saturation: 3.75 with 2 fraction bits is 15.
    let mut fp = 0;
    assert_eq!(to_int(F32, 3.75f32.to_bits() as u64, true, 16, 2, Round::Zero, &mut fp), 15);
    assert_eq!(to_int(F32, 1.0e6f32.to_bits() as u64, true, 16, 2, Round::Zero, &mut fp), 32767);
    assert_eq!(to_int(F32, (-1.0e6f32).to_bits() as u64, true, 16, 2, Round::Zero, &mut fp), 0xffff_8000);
    assert_eq!(fp & IOC, IOC);
}

#[test]
fn integer_to_float_conversions() {
    let mut g = Rng(0x9e37_79b9_7f4a_7c15);
    for _ in 0..100_000 {
        let v = g.next() as u32;
        let mut fp = 0;
        let (neg, mag) = if (v as i32) < 0 { (true, (v as i32).unsigned_abs() as u64) } else { (false, v as u64) };
        assert_eq!(from_int(F32, neg, mag, 0, Round::Nearest, &mut fp) as u32, (v as i32 as f32).to_bits());
        assert_eq!(from_int(F32, false, v as u64, 0, Round::Nearest, &mut fp) as u32, (v as f32).to_bits());
        assert_eq!(from_int(F64, neg, mag, 0, Round::Nearest, &mut fp), (v as i32 as f64).to_bits());
    }
    let mut fp = 0;
    // 16,777,217 is not representable: ties-to-even gives ...216, rounding up gives ...218.
    assert_eq!(from_int(F32, false, 16_777_217, 0, Round::Nearest, &mut fp) as u32, 16_777_216f32.to_bits());
    assert_eq!(fp, IXC);
    assert_eq!(from_int(F32, false, 16_777_217, 0, Round::PlusInf, &mut fp) as u32, 16_777_218f32.to_bits());
    assert_eq!(from_int(F32, false, 5, 1, Round::Nearest, &mut fp) as u32, 2.5f32.to_bits());
}

#[test]
fn round_to_integral_matches_rust() {
    let mut g = Rng(0x1357_2468_acef_bdf1);
    for _ in 0..200_000 {
        let a = g.f32();
        let x = f32::from_bits(a);
        let mut fp = 0;
        let cases = [(Round::Nearest, x.round_ties_even()), (Round::Zero, x.trunc()), (Round::PlusInf, x.ceil()), (Round::MinusInf, x.floor()), (Round::Away, x.round())];
        for (rm, want) in cases {
            let got = f32::from_bits(round_int(F32, a as u64, rm, false, &mut fp) as u32);
            assert!(got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()), "{rm:?} {a:#x}: {got} vs {want}");
        }
    }
    let mut fp = 0;
    round_int(F32, 1.5f32.to_bits() as u64, Round::Zero, true, &mut fp);
    assert_eq!(fp, IXC);
}

#[test]
fn precision_conversions() {
    let mut g = Rng(0x7777_1111_3333_5555);
    for _ in 0..200_000 {
        let a = g.f32();
        let mut fp = 0;
        let d = convert(F32, F64, a as u64, &mut fp);
        assert!(same64(d, f32::from_bits(a) as f64));
        let b = g.f64();
        let s = convert(F64, F32, b, &mut fp) as u32;
        assert!(same32(s, f64::from_bits(b) as f32));
    }
    // Half precision round trip for every non-NaN value.
    for h in 0..=0xffffu16 {
        let mut fp = 0;
        let f = half_to_float(h, F32, &mut fp);
        if (h & 0x7c00) == 0x7c00 && h & 0x3ff != 0 {
            continue;
        }
        assert_eq!(float_to_half(F32, f, &mut fp), h, "half {h:#x}");
        assert_eq!(fp, 0);
        assert_eq!(float_to_half(F64, half_to_float(h, F64, &mut fp), &mut fp), h);
    }
    let mut fp = 0;
    assert_eq!(float_to_half(F32, 1.0f32.to_bits() as u64, &mut fp), 0x3c00);
    assert_eq!(float_to_half(F32, 65520.0f32.to_bits() as u64, &mut fp), 0x7c00); // overflow rounds to infinity
    assert_eq!(fp, OFC | IXC);
    // Alternative half precision: no infinity, saturation with IOC.
    let mut fp = FPSCR_AHP;
    assert_eq!(float_to_half(F32, 1.0e6f32.to_bits() as u64, &mut fp), 0x7fff);
    assert_eq!(fp & IOC, IOC);
    let mut fp = FPSCR_AHP;
    assert_eq!(half_to_float(0x7c00, F32, &mut fp), 65536.0f32.to_bits() as u64); // exponent 31 is a normal number
}

#[test]
fn comparisons_and_min_max() {
    let n = |v: f32| v.to_bits() as u64;
    let mut fp = 0;
    assert_eq!(compare(F32, n(1.0), n(2.0), false, &mut fp), 0x8000_0000);
    assert_eq!(compare(F32, n(2.0), n(1.0), false, &mut fp), 0x2000_0000);
    assert_eq!(compare(F32, n(-0.0), n(0.0), false, &mut fp), 0x6000_0000);
    assert_eq!(compare(F32, n(f32::NAN), n(0.0), false, &mut fp), 0x3000_0000);
    assert_eq!(fp, 0);
    assert_eq!(compare(F32, n(f32::NAN), n(0.0), true, &mut fp), 0x3000_0000);
    assert_eq!(fp, IOC);
    assert_eq!(compare(F32, n(-5.0), n(-3.0), false, &mut fp), 0x8000_0000);
    assert_eq!(compare(F32, n(f32::NEG_INFINITY), n(-3.0), false, &mut fp), 0x8000_0000);
    let mut fp = 0;
    assert_eq!(max_min_num(F32, n(f32::NAN), n(2.0), true, &mut fp), n(2.0));
    assert_eq!(max_min_num(F32, n(2.0), n(f32::NAN), false, &mut fp), n(2.0));
    assert_eq!(fp, 0);
    assert_eq!(max_min_num(F32, n(-0.0), n(0.0), true, &mut fp), n(0.0));
    assert_eq!(max_min_num(F32, n(-0.0), n(0.0), false, &mut fp), n(-0.0));
    assert_eq!(max_min_num(F32, n(1.0), n(3.0), false, &mut fp), n(1.0));
    assert_eq!(max_min_num(F32, 0x7f80_0001, n(3.0), true, &mut fp), 0x7fc0_0001);
    assert_eq!(fp, IOC);
}
