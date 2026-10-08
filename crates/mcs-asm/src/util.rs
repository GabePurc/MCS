//! Small shared helpers: a fast non-cryptographic hasher for the assembler's symbol tables and
//! formatting helpers used in diagnostics.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// FxHash (the rustc hasher): much faster than SipHash for the short identifier keys used here.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        for &b in rest {
            self.add(b as u64);
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;
pub(crate) type FxMap<K, V> = HashMap<K, V, FxBuild>;
pub(crate) type FxSet<K> = HashSet<K, FxBuild>;

/// `0x`-prefixed upper-case hex padded to `digits` (negative values get a leading `-`).
pub(crate) fn hex(v: i64, digits: usize) -> String {
    if v < 0 {
        format!("-0x{:0digits$X}", v.unsigned_abs())
    } else {
        format!("0x{v:0digits$X}")
    }
}

/// File name without its directory part (`/` or `\` separated).
pub(crate) fn basename(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// Length in UTF-16 code units (the unit the original TypeScript tooling used for columns and
/// string lengths). ASCII fast path.
#[inline]
pub(crate) fn utf16_len(s: &str) -> u32 {
    let n = if s.is_ascii() { s.len() } else { s.encode_utf16().count() };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// `Number.prototype.toFixed(1)` for non-negative values: ties round up (Rust rounds exact ties
/// to even). Exact ties of a binary double at one decimal are the values `odd / 4`.
pub(crate) fn to_fixed1(x: f64) -> String {
    let q = x * 4.0;
    if x >= 0.0 && q < 9.0e15 && q.fract() == 0.0 && (q as i64) & 1 == 1 {
        let n = (x * 10.0 + 0.5) as i64;
        return format!("{}.{}", n / 10, n % 10);
    }
    format!("{x:.1}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(hex(0x3f, 2), "0x3F");
        assert_eq!(hex(-5, 2), "-0x05");
        assert_eq!(hex(0x123, 2), "0x123");
        assert_eq!(basename("a/b\\c.inc"), "c.inc");
        assert_eq!(basename("c.inc"), "c.inc");
        assert_eq!(to_fixed1(6.25), "6.3");
        assert_eq!(to_fixed1(0.75), "0.8");
        assert_eq!(to_fixed1(12.5), "12.5");
        assert_eq!(to_fixed1(0.1953125), "0.2");
        assert_eq!(to_fixed1(0.0), "0.0");
        assert_eq!(utf16_len("a\u{1F600}"), 3);
    }
}
