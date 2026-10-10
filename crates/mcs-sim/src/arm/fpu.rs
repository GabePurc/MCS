//! IEEE-754 arithmetic of the ARMv7-M floating-point extensions (FPv4-SP and FPv5-D16) with the
//! exact FPSCR behaviour of the architecture: rounding modes, flush-to-zero (FZ), default NaN
//! (DN), alternative half precision (AHP) and the cumulative exception flags.
//!
//! References: ARM DDI 0403E.e section A2.7 (floating-point pseudocode: `FPUnpack`, `FPRound`,
//! `FPProcessNaNs`, `FPAdd`, `FPMul`, `FPDiv`, `FPSqrt`, `FPMulAdd`, `FPCompare`, `FPToFixed`,
//! `FixedToFP`, `FPRoundInt`, `FPMaxNum`) and B1.4.7 (FPSCR); Cortex-M4 TRM (ARM DDI 0439B) and
//! Cortex-M7 TRM (ARM DDI 0489) for the implemented FPU options.
//!
//! The reference implementation is an exact integer soft-float ([`Fmt`]-generic, `u128`
//! intermediates, one sticky bit): every operation produces the exact result as
//! `(sign, exponent, 128-bit mantissa, sticky)` and `round` applies `FPRound` once. The
//! hot cases (round-to-nearest, no flush-to-zero, finite normal operands) take a native `f32` /
//! `f64` fast path whose inexact flag is derived from error-free transformations; both paths are
//! cross-checked against each other by the unit tests.

pub const IOC: u32 = 1;
pub const DZC: u32 = 1 << 1;
pub const OFC: u32 = 1 << 2;
pub const UFC: u32 = 1 << 3;
pub const IXC: u32 = 1 << 4;
pub const IDC: u32 = 1 << 7;
pub const FPSCR_FZ: u32 = 1 << 24;
pub const FPSCR_DN: u32 = 1 << 25;
pub const FPSCR_AHP: u32 = 1 << 26;
pub const FPSCR_RMODE: u32 = 3 << 22;
/// Writable FPSCR bits: NZCV, AHP, DN, FZ, RMode and the cumulative flags (the exception trap
/// enables are not implemented on Cortex-M4/M7 and read as zero).
pub const FPSCR_WMASK: u32 = 0xf7c0_009f;

/// Rounding mode (FPSCR.RMode encodings 0-3, plus ties-away for VRINTA / VCVTA).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Round {
    Nearest,
    PlusInf,
    MinusInf,
    Zero,
    Away,
}

impl Round {
    #[inline]
    pub fn from_fpscr(fpscr: u32) -> Round {
        match (fpscr >> 22) & 3 {
            0 => Round::Nearest,
            1 => Round::PlusInf,
            2 => Round::MinusInf,
            _ => Round::Zero,
        }
    }
}

/// Floating-point format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fmt {
    pub ebits: u32,
    pub fbits: u32,
}

pub const F16: Fmt = Fmt { ebits: 5, fbits: 10 };
pub const F32: Fmt = Fmt { ebits: 8, fbits: 23 };
pub const F64: Fmt = Fmt { ebits: 11, fbits: 52 };

impl Fmt {
    #[inline]
    const fn bias(self) -> i32 {
        (1 << (self.ebits - 1)) - 1
    }
    #[inline]
    const fn min_exp(self) -> i32 {
        1 - self.bias()
    }
    #[inline]
    const fn sign_bit(self) -> u64 {
        1 << (self.ebits + self.fbits)
    }
    #[inline]
    const fn exp_mask(self) -> u64 {
        ((1u64 << self.ebits) - 1) << self.fbits
    }
    #[inline]
    const fn frac_mask(self) -> u64 {
        (1u64 << self.fbits) - 1
    }
    #[inline]
    const fn quiet_bit(self) -> u64 {
        1 << (self.fbits - 1)
    }
    #[inline]
    const fn zero(self, sign: bool) -> u64 {
        if sign {
            self.sign_bit()
        } else {
            0
        }
    }
    #[inline]
    const fn inf(self, sign: bool) -> u64 {
        self.zero(sign) | self.exp_mask()
    }
    #[inline]
    const fn max_normal(self, sign: bool) -> u64 {
        self.zero(sign) | (self.exp_mask() - (1 << self.fbits)) | self.frac_mask()
    }
    /// The default quiet NaN (positive, empty payload).
    #[inline]
    pub const fn default_nan(self) -> u64 {
        self.exp_mask() | self.quiet_bit()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cls {
    Zero,
    Norm,
    Inf,
    QNan,
    SNan,
}

/// An unpacked operand: a normal number is `mant / 2^63 * 2^exp` with `mant` bit 63 set.
#[derive(Clone, Copy)]
struct Unp {
    cls: Cls,
    sign: bool,
    exp: i32,
    mant: u64,
    bits: u64,
}

impl Unp {
    #[inline]
    fn is_nan(&self) -> bool {
        matches!(self.cls, Cls::QNan | Cls::SNan)
    }
}

/// FPUnpack. With `flush` (FPSCR.FZ and not half precision) a denormal operand becomes zero and
/// the input-denormal flag IDC is set.
#[inline]
fn unpack(f: Fmt, bits: u64, flush: bool, fpscr: &mut u32) -> Unp {
    let sign = bits & f.sign_bit() != 0;
    let e = ((bits >> f.fbits) & ((1u64 << f.ebits) - 1)) as i32;
    let frac = bits & f.frac_mask();
    let mut u = Unp { cls: Cls::Zero, sign, exp: 0, mant: 0, bits };
    if e == (1 << f.ebits) - 1 {
        u.cls = if frac == 0 {
            Cls::Inf
        } else if frac & f.quiet_bit() != 0 {
            Cls::QNan
        } else {
            Cls::SNan
        };
    } else if e == 0 {
        if frac != 0 {
            if flush {
                *fpscr |= IDC;
            } else {
                let lz = frac.leading_zeros();
                u.cls = Cls::Norm;
                u.mant = frac << lz;
                u.exp = 63 - lz as i32 + f.min_exp() - f.fbits as i32;
            }
        }
    } else {
        u.cls = Cls::Norm;
        u.mant = ((1u64 << f.fbits) | frac) << (63 - f.fbits);
        u.exp = e - f.bias();
    }
    u
}

#[inline]
fn flush_on(f: Fmt, fpscr: u32) -> bool {
    fpscr & FPSCR_FZ != 0 && f.ebits != 5
}

/// FPRound for the exact value `(-1)^sign * (m / 2^127) * 2^exp + sticky` (bit 127 of `m` set;
/// `sticky` stands for non-zero bits below `m`). Sets UFC / OFC / IXC (and IOC for AHP
/// overflow). Returns the encoded result.
#[allow(clippy::too_many_arguments)]
fn round(f: Fmt, sign: bool, exp: i32, m: u128, sticky: bool, rm: Round, ahp: bool, fpscr: &mut u32) -> u64 {
    debug_assert!(m >> 127 == 1);
    let min_exp = f.min_exp();
    if flush_on(f, *fpscr) && exp < min_exp {
        *fpscr |= UFC;
        return f.zero(sign);
    }
    let mut biased = (exp - min_exp + 1).max(0);
    let mut shift = 127 - f.fbits as i32;
    if biased == 0 {
        shift += min_exp - exp;
    }
    // Integer mantissa, remainder and its position relative to one half.
    let (mut int_mant, err_nonzero, above_half, exactly_half);
    if shift > 128 {
        int_mant = 0u64;
        err_nonzero = true;
        above_half = false;
        exactly_half = false;
    } else if shift == 128 {
        int_mant = 0;
        err_nonzero = true;
        let half = 1u128 << 127;
        above_half = m > half || (m == half && sticky);
        exactly_half = m == half && !sticky;
    } else {
        let sh = shift as u32;
        int_mant = (m >> sh) as u64;
        let rem = m & ((1u128 << sh) - 1);
        let half = 1u128 << (sh - 1);
        err_nonzero = rem != 0 || sticky;
        above_half = rem > half || (rem == half && sticky);
        exactly_half = rem == half && !sticky;
    }
    if biased == 0 && err_nonzero {
        *fpscr |= UFC;
    }
    let (round_up, overflow_to_inf) = match rm {
        Round::Nearest | Round::Away => (above_half || (exactly_half && (rm == Round::Away || int_mant & 1 == 1)), true),
        Round::PlusInf => (err_nonzero && !sign, !sign),
        Round::MinusInf => (err_nonzero && sign, sign),
        Round::Zero => (false, false),
    };
    if round_up {
        int_mant += 1;
        if biased != 0 {
            if int_mant == 1 << (f.fbits + 1) {
                biased += 1;
                int_mant >>= 1;
            }
        } else if int_mant == 1 << f.fbits {
            biased = 1;
        }
    }
    let limit = (1i32 << f.ebits) - if ahp { 0 } else { 1 };
    if biased >= limit {
        *fpscr |= if ahp { IOC } else { OFC | IXC };
        return if ahp {
            f.zero(sign) | f.exp_mask() | f.frac_mask()
        } else if overflow_to_inf {
            f.inf(sign)
        } else {
            f.max_normal(sign)
        };
    }
    if err_nonzero {
        *fpscr |= IXC;
    }
    f.zero(sign) | (biased as u64) << f.fbits | (int_mant & f.frac_mask())
}

/// Convenience: round with the FPSCR rounding mode.
#[inline]
fn round_fpscr(f: Fmt, sign: bool, exp: i32, m: u128, sticky: bool, fpscr: &mut u32) -> u64 {
    let rm = Round::from_fpscr(*fpscr);
    round(f, sign, exp, m, sticky, rm, false, fpscr)
}

/// FPProcessNaN for a NaN operand (quiets a signalling NaN; DN selects the default NaN).
#[inline]
fn propagate(f: Fmt, u: &Unp, fpscr: u32) -> u64 {
    if fpscr & FPSCR_DN != 0 {
        f.default_nan()
    } else {
        u.bits | f.quiet_bit()
    }
}

/// FPProcessNaNs for two operands.
#[inline]
fn nan2(f: Fmt, a: &Unp, b: &Unp, fpscr: &mut u32) -> Option<u64> {
    if !(a.is_nan() || b.is_nan()) {
        return None;
    }
    if a.cls == Cls::SNan || b.cls == Cls::SNan {
        *fpscr |= IOC;
    }
    let pick = if a.cls == Cls::SNan {
        a
    } else if b.cls == Cls::SNan {
        b
    } else if a.cls == Cls::QNan {
        a
    } else {
        b
    };
    Some(propagate(f, pick, *fpscr))
}

/// FPProcessNaNs3 (priority: signalling NaNs a, b, c, then quiet NaNs a, b, c).
fn nan3(f: Fmt, a: &Unp, b: &Unp, c: &Unp, fpscr: &mut u32) -> Option<u64> {
    let all = [a, b, c];
    if !all.iter().any(|u| u.is_nan()) {
        return None;
    }
    if all.iter().any(|u| u.cls == Cls::SNan) {
        *fpscr |= IOC;
    }
    let pick = all.iter().find(|u| u.cls == Cls::SNan).or_else(|| all.iter().find(|u| u.cls == Cls::QNan)).unwrap();
    Some(propagate(f, pick, *fpscr))
}

/// Exact sum of two magnitudes with mantissas `m` (bit 126 set, value `m / 2^126 * 2^exp`).
/// Returns `(sign, exp, m (bit 127 set), sticky)`, or `None` for an exact zero.
fn add_mag(sa: bool, ea: i32, ma: u128, sb: bool, eb: i32, mb: u128) -> Option<(bool, i32, u128, bool)> {
    // Order by magnitude.
    let (sbig, ebig, mbig, ssmall, esmall, msmall) = if (ea, ma) >= (eb, mb) { (sa, ea, ma, sb, eb, mb) } else { (sb, eb, mb, sa, ea, ma) };
    let d = (ebig - esmall) as u32;
    let (aligned, sticky) = if d >= 127 { (0, true) } else { (msmall >> d, msmall & ((1u128 << d) - 1) != 0) };
    if sbig == ssmall {
        let sum = mbig + aligned;
        let lz = sum.leading_zeros();
        Some((sbig, ebig + 1 - lz as i32, sum << lz, sticky))
    } else {
        let diff = mbig - aligned - sticky as u128;
        if diff == 0 {
            return None;
        }
        let lz = diff.leading_zeros();
        Some((sbig, ebig + 1 - lz as i32, diff << lz, sticky))
    }
}

/// Exact product of two unpacked normal numbers: `(exp, m with bit 127 set)`.
#[inline]
fn mul_mag(a: &Unp, b: &Unp) -> (i32, u128) {
    let p = a.mant as u128 * b.mant as u128;
    if p >> 127 != 0 {
        (a.exp + b.exp + 1, p)
    } else {
        (a.exp + b.exp, p << 1)
    }
}

/// FPAdd / FPSub (`sub` negates the second operand).
pub fn add_soft(f: Fmt, a: u64, b: u64, sub: bool, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, a, flush, fpscr);
    let ub = unpack(f, b, flush, fpscr);
    if let Some(r) = nan2(f, &ua, &ub, fpscr) {
        return r;
    }
    let sb = ub.sign ^ sub;
    let rm = Round::from_fpscr(*fpscr);
    let zero_sign = rm == Round::MinusInf;
    match (ua.cls, ub.cls) {
        (Cls::Inf, Cls::Inf) => {
            if ua.sign != sb {
                *fpscr |= IOC;
                f.default_nan()
            } else {
                f.inf(ua.sign)
            }
        }
        (Cls::Inf, _) => f.inf(ua.sign),
        (_, Cls::Inf) => f.inf(sb),
        (Cls::Zero, Cls::Zero) => f.zero(if ua.sign == sb { ua.sign } else { zero_sign }),
        _ => {
            // At least one finite non-zero operand. A zero operand contributes nothing.
            let ma = if ua.cls == Cls::Zero { None } else { Some((ua.exp, (ua.mant as u128) << 63)) };
            let mb = if ub.cls == Cls::Zero { None } else { Some((ub.exp, (ub.mant as u128) << 63)) };
            let r = match (ma, mb) {
                (Some((ea, ma)), Some((eb, mb))) => add_mag(ua.sign, ea, ma, sb, eb, mb),
                (Some((ea, ma)), None) => Some((ua.sign, ea, ma << 1, false)),
                (None, Some((eb, mb))) => Some((sb, eb, mb << 1, false)),
                (None, None) => None,
            };
            match r {
                Some((s, e, m, st)) => round(f, s, e, m, st, rm, false, fpscr),
                None => f.zero(zero_sign),
            }
        }
    }
}

/// FPMul.
pub fn mul_soft(f: Fmt, a: u64, b: u64, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, a, flush, fpscr);
    let ub = unpack(f, b, flush, fpscr);
    if let Some(r) = nan2(f, &ua, &ub, fpscr) {
        return r;
    }
    let sign = ua.sign ^ ub.sign;
    match (ua.cls, ub.cls) {
        (Cls::Inf, Cls::Zero) | (Cls::Zero, Cls::Inf) => {
            *fpscr |= IOC;
            f.default_nan()
        }
        (Cls::Inf, _) | (_, Cls::Inf) => f.inf(sign),
        (Cls::Zero, _) | (_, Cls::Zero) => f.zero(sign),
        _ => {
            let (e, m) = mul_mag(&ua, &ub);
            round_fpscr(f, sign, e, m, false, fpscr)
        }
    }
}

/// FPDiv.
pub fn div_soft(f: Fmt, a: u64, b: u64, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, a, flush, fpscr);
    let ub = unpack(f, b, flush, fpscr);
    if let Some(r) = nan2(f, &ua, &ub, fpscr) {
        return r;
    }
    let sign = ua.sign ^ ub.sign;
    match (ua.cls, ub.cls) {
        (Cls::Inf, Cls::Inf) | (Cls::Zero, Cls::Zero) => {
            *fpscr |= IOC;
            f.default_nan()
        }
        (Cls::Inf, _) | (_, Cls::Zero) => {
            if ua.cls != Cls::Inf {
                *fpscr |= DZC;
            }
            f.inf(sign)
        }
        (Cls::Zero, _) | (_, Cls::Inf) => f.zero(sign),
        _ => {
            let n = (ua.mant as u128) << 64;
            let d = ub.mant as u128;
            let q = n / d;
            let sticky = !n.is_multiple_of(d);
            let (e, m) = if q >> 64 != 0 { (ua.exp - ub.exp, q << 63) } else { (ua.exp - ub.exp - 1, q << 64) };
            round_fpscr(f, sign, e, m, sticky, fpscr)
        }
    }
}

/// Floor of the square root of `x` and whether the root is inexact.
fn isqrt(x: u128) -> (u64, bool) {
    let mut rem = x;
    let mut root = 0u128;
    let mut bit = 1u128 << 126;
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    (root as u64, rem != 0)
}

/// FPSqrt.
pub fn sqrt_soft(f: Fmt, a: u64, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let u = unpack(f, a, flush, fpscr);
    match u.cls {
        Cls::QNan | Cls::SNan => {
            if u.cls == Cls::SNan {
                *fpscr |= IOC;
            }
            propagate(f, &u, *fpscr)
        }
        Cls::Zero => f.zero(u.sign),
        Cls::Inf if !u.sign => f.inf(false),
        _ if u.sign => {
            *fpscr |= IOC;
            f.default_nan()
        }
        _ => {
            let odd = u.exp & 1;
            let x = (u.mant as u128) << (63 + odd);
            let (r, inexact) = isqrt(x);
            let e = (u.exp - odd) >> 1;
            round_fpscr(f, false, e, (r as u128) << 64, inexact, fpscr)
        }
    }
}

/// FPMulAdd: `addend + op1 * op2` with a single rounding.
pub fn fma_soft(f: Fmt, addend: u64, op1: u64, op2: u64, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, addend, flush, fpscr);
    let u1 = unpack(f, op1, flush, fpscr);
    let u2 = unpack(f, op2, flush, fpscr);
    let inf1 = u1.cls == Cls::Inf;
    let inf2 = u2.cls == Cls::Inf;
    let zero1 = u1.cls == Cls::Zero;
    let zero2 = u2.cls == Cls::Zero;
    let invalid_product = (inf1 && zero2) || (zero1 && inf2);
    if let Some(r) = nan3(f, &ua, &u1, &u2, fpscr) {
        if ua.cls == Cls::QNan && invalid_product {
            *fpscr |= IOC;
            return f.default_nan();
        }
        return r;
    }
    let rm = Round::from_fpscr(*fpscr);
    let zero_sign = rm == Round::MinusInf;
    let sign_p = u1.sign ^ u2.sign;
    let inf_p = inf1 || inf2;
    let zero_p = zero1 || zero2;
    let inf_a = ua.cls == Cls::Inf;
    if invalid_product || (inf_a && inf_p && ua.sign != sign_p) {
        *fpscr |= IOC;
        return f.default_nan();
    }
    if inf_a {
        return f.inf(ua.sign);
    }
    if inf_p {
        return f.inf(sign_p);
    }
    if ua.cls == Cls::Zero && zero_p {
        return f.zero(if ua.sign == sign_p { ua.sign } else { zero_sign });
    }
    let prod = if zero_p {
        None
    } else {
        let (e, m) = mul_mag(&u1, &u2);
        Some((e, m >> 1)) // bit 126 set; the product has at least 22 trailing zero bits
    };
    let add = if ua.cls == Cls::Zero { None } else { Some((ua.exp, (ua.mant as u128) << 63)) };
    let r = match (prod, add) {
        (Some((ep, mp)), Some((ea, ma))) => add_mag(sign_p, ep, mp, ua.sign, ea, ma),
        (Some((ep, mp)), None) => Some((sign_p, ep, mp << 1, false)),
        (None, Some((ea, ma))) => Some((ua.sign, ea, ma << 1, false)),
        (None, None) => None,
    };
    match r {
        Some((s, e, m, st)) => round(f, s, e, m, st, rm, false, fpscr),
        None => f.zero(zero_sign),
    }
}

/// FPCompare: the NZCV nibble (in bits 31:28 of the result).
pub fn compare(f: Fmt, a: u64, b: u64, quiet_nan_exc: bool, fpscr: &mut u32) -> u32 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, a, flush, fpscr);
    let ub = unpack(f, b, flush, fpscr);
    if ua.is_nan() || ub.is_nan() {
        if ua.cls == Cls::SNan || ub.cls == Cls::SNan || quiet_nan_exc {
            *fpscr |= IOC;
        }
        return 0x3000_0000;
    }
    match cmp_values(&ua, &ub) {
        std::cmp::Ordering::Equal => 0x6000_0000,
        std::cmp::Ordering::Less => 0x8000_0000,
        std::cmp::Ordering::Greater => 0x2000_0000,
    }
}

/// Orders two non-NaN operands by value (+0 == -0).
fn cmp_values(a: &Unp, b: &Unp) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let key = |u: &Unp| -> (i8, i32, u64) {
        // (class rank, exponent, mantissa) for the magnitude; zero < normal < inf
        match u.cls {
            Cls::Zero => (0, 0, 0),
            Cls::Norm => (1, u.exp, u.mant),
            _ => (2, 0, 0),
        }
    };
    let (ka, kb) = (key(a), key(b));
    if ka.0 == 0 && kb.0 == 0 {
        return Equal;
    }
    match (a.sign && ka.0 != 0, b.sign && kb.0 != 0) {
        (false, true) => Greater,
        (true, false) => Less,
        (false, false) => ka.cmp(&kb),
        (true, true) => kb.cmp(&ka),
    }
}

/// FPMaxNum / FPMinNum (IEEE 754-2008 maxNum / minNum): a lone quiet NaN is treated as missing
/// data, so the other operand wins.
pub fn max_min_num(f: Fmt, a: u64, b: u64, is_max: bool, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let ua = unpack(f, a, flush, fpscr);
    let ub = unpack(f, b, flush, fpscr);
    let (a_q, b_q) = (ua.cls == Cls::QNan, ub.cls == Cls::QNan);
    if a_q != b_q {
        let other = if a_q { &ub } else { &ua };
        if other.cls == Cls::SNan {
            *fpscr |= IOC;
            return propagate(f, other, *fpscr);
        }
        return value_bits(f, other);
    }
    if let Some(r) = nan2(f, &ua, &ub, fpscr) {
        return r;
    }
    use std::cmp::Ordering::*;
    let pick_a = match cmp_values(&ua, &ub) {
        Greater => is_max,
        Less => !is_max,
        Equal => {
            // +0 and -0 compare equal but order as -0 < +0.
            if ua.cls == Cls::Zero && ub.cls == Cls::Zero && ua.sign != ub.sign {
                return f.zero(!is_max);
            }
            true
        }
    };
    value_bits(f, if pick_a { &ua } else { &ub })
}

/// Encoding of a non-NaN unpacked operand (flushed denormals are zero).
fn value_bits(f: Fmt, u: &Unp) -> u64 {
    if u.cls == Cls::Zero {
        f.zero(u.sign)
    } else {
        u.bits
    }
}

/// FPToFixed: float to a signed / unsigned integer of `nbits` (16 or 32) bits with `fbits`
/// fraction bits, saturating. NaN converts to 0. Returns the 32-bit register value (sign- or
/// zero-extended).
pub fn to_int(f: Fmt, a: u64, signed: bool, nbits: u32, fbits: u32, rm: Round, fpscr: &mut u32) -> u32 {
    let flush = flush_on(f, *fpscr);
    let u = unpack(f, a, flush, fpscr);
    let (max_pos, max_neg_mag): (u64, u64) = if signed { ((1u64 << (nbits - 1)) - 1, 1u64 << (nbits - 1)) } else { ((1u64 << nbits) - 1, 0) };
    let sat = |sign: bool, fpscr: &mut u32| -> u32 {
        *fpscr |= IOC;
        if sign {
            if signed {
                (max_neg_mag as i64).wrapping_neg() as u32
            } else {
                0
            }
        } else {
            max_pos as u32
        }
    };
    match u.cls {
        Cls::QNan | Cls::SNan => {
            *fpscr |= IOC;
            0
        }
        Cls::Zero => 0,
        Cls::Inf => sat(u.sign, fpscr),
        Cls::Norm => {
            let e = u.exp + fbits as i32;
            if e >= 40 {
                return sat(u.sign, fpscr);
            }
            let m = u.mant as u128;
            let shift = 63 - e;
            let (mut int, rem_nonzero, above, tie);
            if shift <= 0 {
                int = (m << (-shift) as u32) as u64;
                rem_nonzero = false;
                above = false;
                tie = false;
            } else if shift > 127 {
                int = 0;
                rem_nonzero = true;
                above = false;
                tie = false;
            } else {
                let sh = shift as u32;
                int = (m >> sh) as u64;
                let rem = m & ((1u128 << sh) - 1);
                let half = 1u128 << (sh - 1);
                rem_nonzero = rem != 0;
                above = rem > half;
                tie = rem == half;
            }
            let up = match rm {
                Round::Nearest => above || (tie && int & 1 == 1),
                Round::Away => above || tie,
                Round::Zero => false,
                Round::PlusInf => rem_nonzero && !u.sign,
                Round::MinusInf => rem_nonzero && u.sign,
            };
            if up {
                int += 1;
            }
            let ok = if u.sign { int <= max_neg_mag } else { int <= max_pos };
            if !ok {
                return sat(u.sign, fpscr);
            }
            if rem_nonzero {
                *fpscr |= IXC;
            }
            if u.sign {
                (int as i64).wrapping_neg() as u32
            } else {
                int as u32
            }
        }
    }
}

/// FixedToFP: the integer `mag` (with sign `neg`) scaled by `2^-fbits`, rounded with `rm`.
pub fn from_int(f: Fmt, neg: bool, mag: u64, fbits: u32, rm: Round, fpscr: &mut u32) -> u64 {
    if mag == 0 {
        return f.zero(false);
    }
    let lz = mag.leading_zeros();
    let m = (mag as u128) << (64 + lz);
    round(f, neg, 63 - lz as i32 - fbits as i32, m, false, rm, false, fpscr)
}

/// FPRoundInt: rounds to an integral value in the same format.
pub fn round_int(f: Fmt, a: u64, rm: Round, exact: bool, fpscr: &mut u32) -> u64 {
    let flush = flush_on(f, *fpscr);
    let u = unpack(f, a, flush, fpscr);
    match u.cls {
        Cls::QNan | Cls::SNan => {
            if u.cls == Cls::SNan {
                *fpscr |= IOC;
            }
            propagate(f, &u, *fpscr)
        }
        Cls::Inf => f.inf(u.sign),
        Cls::Zero => f.zero(u.sign),
        Cls::Norm => {
            if u.exp >= f.fbits as i32 {
                return a;
            }
            let shift = 63 - u.exp;
            let (mut int, nonzero, above, tie);
            if shift > 127 {
                int = 0u64;
                nonzero = true;
                above = false;
                tie = false;
            } else {
                let sh = shift as u32;
                int = ((u.mant as u128) >> sh) as u64;
                let rem = (u.mant as u128) & ((1u128 << sh) - 1);
                let half = 1u128 << (sh - 1);
                nonzero = rem != 0;
                above = rem > half;
                tie = rem == half;
            }
            let up = match rm {
                Round::Nearest => above || (tie && int & 1 == 1),
                Round::Away => above || tie,
                Round::Zero => false,
                Round::PlusInf => nonzero && !u.sign,
                Round::MinusInf => nonzero && u.sign,
            };
            if up {
                int += 1;
            }
            if nonzero && exact {
                *fpscr |= IXC;
            }
            if int == 0 {
                f.zero(u.sign)
            } else {
                let lz = int.leading_zeros();
                round(f, u.sign, 63 - lz as i32, (int as u128) << (64 + lz), false, Round::Nearest, false, fpscr)
            }
        }
    }
}

/// FPConvert between single and double precision.
pub fn convert(from: Fmt, to: Fmt, a: u64, fpscr: &mut u32) -> u64 {
    let flush = flush_on(from, *fpscr);
    let u = unpack(from, a, flush, fpscr);
    match u.cls {
        Cls::QNan | Cls::SNan => {
            if u.cls == Cls::SNan {
                *fpscr |= IOC;
            }
            if *fpscr & FPSCR_DN != 0 {
                return to.default_nan();
            }
            let frac = u.bits & from.frac_mask();
            let payload = if to.fbits > from.fbits { frac << (to.fbits - from.fbits) } else { frac >> (from.fbits - to.fbits) };
            to.zero(u.sign) | to.exp_mask() | to.quiet_bit() | (payload & to.frac_mask())
        }
        Cls::Inf => to.inf(u.sign),
        Cls::Zero => to.zero(u.sign),
        Cls::Norm => round_fpscr(to, u.sign, u.exp, (u.mant as u128) << 64, false, fpscr),
    }
}

/// Half precision (IEEE or ARM alternative format per FPSCR.AHP) to single / double.
pub fn half_to_float(h: u16, to: Fmt, fpscr: &mut u32) -> u64 {
    let ahp = *fpscr & FPSCR_AHP != 0;
    let sign = h & 0x8000 != 0;
    let e = ((h >> 10) & 0x1f) as i32;
    let frac = (h & 0x3ff) as u64;
    if !ahp && e == 31 {
        if frac == 0 {
            return to.inf(sign);
        }
        if frac & 0x200 == 0 {
            *fpscr |= IOC;
        }
        if *fpscr & FPSCR_DN != 0 {
            return to.default_nan();
        }
        return to.zero(sign) | to.exp_mask() | to.quiet_bit() | ((frac & 0x1ff) << (to.fbits - 10));
    }
    if e == 0 && frac == 0 {
        return to.zero(sign);
    }
    let (exp, mant) = if e == 0 {
        let lz = frac.leading_zeros();
        (63 - lz as i32 - 24, frac << lz)
    } else {
        (e - 15, (0x400 | frac) << 53)
    };
    round(to, sign, exp, (mant as u128) << 64, false, Round::Nearest, false, fpscr)
}

/// Single / double precision to half precision (FPSCR rounding mode, AHP selects the format).
pub fn float_to_half(from: Fmt, a: u64, fpscr: &mut u32) -> u16 {
    let ahp = *fpscr & FPSCR_AHP != 0;
    let flush = flush_on(from, *fpscr);
    let u = unpack(from, a, flush, fpscr);
    let sign_h = if u.sign { 0x8000u16 } else { 0 };
    match u.cls {
        Cls::QNan | Cls::SNan => {
            if u.cls == Cls::SNan || ahp {
                *fpscr |= IOC;
            }
            if ahp {
                0
            } else if *fpscr & FPSCR_DN != 0 {
                0x7e00
            } else {
                let frac = u.bits & from.frac_mask();
                sign_h | 0x7c00 | 0x200 | ((frac >> (from.fbits - 9)) & 0x1ff) as u16
            }
        }
        Cls::Inf => {
            if ahp {
                *fpscr |= IOC;
                sign_h | 0x7fff
            } else {
                sign_h | 0x7c00
            }
        }
        Cls::Zero => sign_h,
        Cls::Norm => {
            let rm = Round::from_fpscr(*fpscr);
            round(F16, u.sign, u.exp, (u.mant as u128) << 64, false, rm, ahp, fpscr) as u16
        }
    }
}

// ---- native fast paths ------------------------------------------------------------------------

/// True when the fast paths are valid: round-to-nearest and flush-to-zero off.
#[inline(always)]
pub fn fast_ok(fpscr: u32) -> bool {
    fpscr & (FPSCR_FZ | FPSCR_RMODE) == 0
}

#[inline(always)]
pub fn add32(a: u32, b: u32, sub: bool, fpscr: &mut u32) -> u32 {
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    let y = if sub { -y } else { y };
    let s = x + y;
    if fast_ok(*fpscr) && x.is_finite() && y.is_finite() && s.is_finite() {
        // TwoSum: the rounding error of the sum is exactly representable.
        let bb = s - x;
        let err = (x - (s - bb)) + (y - bb);
        if err != 0.0 {
            *fpscr |= IXC;
        }
        // A zero result of two non-zero operands is +0 in round-to-nearest, as the native add gives.
        return s.to_bits();
    }
    add_soft(F32, a as u64, b as u64, sub, fpscr) as u32
}

#[inline(always)]
pub fn mul32(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    let p = x as f64 * y as f64; // exact
    let r = p as f32;
    if fast_ok(*fpscr) && x.is_finite() && y.is_finite() && (p.abs() >= f32::MIN_POSITIVE as f64 || p == 0.0) && r.is_finite() {
        if r as f64 != p {
            *fpscr |= IXC;
        }
        return (x * y).to_bits();
    }
    mul_soft(F32, a as u64, b as u64, fpscr) as u32
}

#[inline(always)]
pub fn div32(a: u32, b: u32, fpscr: &mut u32) -> u32 {
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    let r = x / y;
    let ra = r.abs();
    if fast_ok(*fpscr) && (1e-15..=1e15).contains(&x.abs()) && (1e-15..=1e15).contains(&y.abs()) && (1e-30..=1e30).contains(&ra) {
        if (-r).mul_add(y, x) != 0.0 {
            *fpscr |= IXC;
        }
        return r.to_bits();
    }
    div_soft(F32, a as u64, b as u64, fpscr) as u32
}

#[inline(always)]
pub fn sqrt32(a: u32, fpscr: &mut u32) -> u32 {
    let x = f32::from_bits(a);
    let r = x.sqrt();
    if fast_ok(*fpscr) && (1e-15..=1e30).contains(&x) {
        if (-r).mul_add(r, x) != 0.0 {
            *fpscr |= IXC;
        }
        return r.to_bits();
    }
    sqrt_soft(F32, a as u64, fpscr) as u32
}

#[inline(always)]
pub fn add64(a: u64, b: u64, sub: bool, fpscr: &mut u32) -> u64 {
    let (x, y) = (f64::from_bits(a), f64::from_bits(b));
    let y = if sub { -y } else { y };
    let s = x + y;
    if fast_ok(*fpscr) && x.is_finite() && y.is_finite() && s.is_finite() {
        let bb = s - x;
        let err = (x - (s - bb)) + (y - bb);
        if err != 0.0 {
            *fpscr |= IXC;
        }
        return s.to_bits();
    }
    add_soft(F64, a, b, sub, fpscr)
}

#[inline(always)]
pub fn mul64(a: u64, b: u64, fpscr: &mut u32) -> u64 {
    let (x, y) = (f64::from_bits(a), f64::from_bits(b));
    let r = x * y;
    let ra = r.abs();
    if fast_ok(*fpscr) && (1e-250..=1e300).contains(&ra) && x.is_finite() && y.is_finite() && x.abs() >= f64::MIN_POSITIVE && y.abs() >= f64::MIN_POSITIVE {
        if x.mul_add(y, -r) != 0.0 {
            *fpscr |= IXC;
        }
        return r.to_bits();
    }
    if fast_ok(*fpscr) && (x == 0.0 || y == 0.0) && x.is_finite() && y.is_finite() {
        return r.to_bits();
    }
    mul_soft(F64, a, b, fpscr)
}

#[inline(always)]
pub fn div64(a: u64, b: u64, fpscr: &mut u32) -> u64 {
    let (x, y) = (f64::from_bits(a), f64::from_bits(b));
    let r = x / y;
    let ra = r.abs();
    if fast_ok(*fpscr) && (1e-150..=1e150).contains(&x.abs()) && (1e-150..=1e150).contains(&y.abs()) && (1e-250..=1e250).contains(&ra) {
        if (-r).mul_add(y, x) != 0.0 {
            *fpscr |= IXC;
        }
        return r.to_bits();
    }
    div_soft(F64, a, b, fpscr)
}

#[inline(always)]
pub fn sqrt64(a: u64, fpscr: &mut u32) -> u64 {
    let x = f64::from_bits(a);
    let r = x.sqrt();
    if fast_ok(*fpscr) && (1e-150..=1e300).contains(&x) {
        if (-r).mul_add(r, x) != 0.0 {
            *fpscr |= IXC;
        }
        return r.to_bits();
    }
    sqrt_soft(F64, a, fpscr)
}

#[cfg(test)]
#[path = "fpu_tests.rs"]
mod tests;
