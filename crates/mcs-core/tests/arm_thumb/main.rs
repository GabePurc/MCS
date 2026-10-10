//! Thumb decoder/disassembler checked against `llvm-objdump --triple=thumbv7em` output (Apple
//! clang) for every ARMv7-M base encoding; see `vectors.rs` (generated, checked in).

mod vectors;

use mcs_core::arm::disasm::Disassembler;
use mcs_core::arm::thumb::{decode, ArmFeatures, Insn, Op};
use vectors::{EXTENSION_VECTORS, LONG_BRANCH_VECTORS, VECTORS};

/// Normalizes objdump / our text: drops `<sym>` parts and comments, converts `0x..` literals to
/// decimal and collapses whitespace.
fn norm(s: &str) -> String {
    let mut s = s.split('@').next().unwrap().to_string();
    while let (Some(a), Some(b)) = (s.find('<'), s.find('>')) {
        if a < b {
            s.replace_range(a..=b, "");
        } else {
            break;
        }
    }
    let mut out = String::new();
    let b = s.as_bytes();
    let mut k = 0;
    while k < b.len() {
        if b[k] == b'0' && k + 1 < b.len() && b[k + 1] == b'x' {
            let mut e = k + 2;
            while e < b.len() && b[e].is_ascii_hexdigit() {
                e += 1;
            }
            let v = u64::from_str_radix(&s[k + 2..e], 16).unwrap();
            out.push_str(&v.to_string());
            k = e;
        } else {
            out.push(b[k] as char);
            k += 1;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run(stream: &[(u32, &[u8], &str)], feat: ArmFeatures) -> Vec<String> {
    let mut d = Disassembler::new(feat);
    let mut bad = Vec::new();
    for (addr, bytes, want) in stream {
        let (len, got) = d.next(bytes, *addr).expect("bytes");
        let ok = len == bytes.len() && norm(&got) == norm(want);
        if !ok {
            bad.push(format!("{:#x} {:02x?}: got `{}` want `{}`", addr, bytes, norm(&got), norm(want)));
        }
    }
    bad
}

#[test]
fn base_isa_matches_llvm_objdump() {
    let bad = run(VECTORS, ArmFeatures::CORTEX_M4F);
    if !bad.is_empty() {
        let n = bad.len();
        let max: usize = std::env::var("ARM_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        panic!("{} of {} mismatches:\n{}", n, VECTORS.len(), bad.iter().take(max).cloned().collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn extension_encodings_are_undefined_in_base() {
    // DSP / FP encodings must not decode to a base instruction (they decode to UNDEF for now).
    for (_, bytes, text) in EXTENSION_VECTORS {
        if *text == "nop" {
            continue; // trailing pad of the generated stream
        }
        let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]);
        let hw2 = if bytes.len() > 2 { u16::from_le_bytes([bytes[2], bytes[3]]) } else { 0 };
        let i = decode(hw1, hw2, ArmFeatures::BASE);
        assert_eq!(i.op, Op::UNDEF, "{} decoded as {:?}", text, i.op);
    }
}

#[test]
fn insn_is_small() {
    assert!(std::mem::size_of::<Insn>() <= 16);
}

#[test]
fn dsp_extend_and_add_decodes_with_dsp_feature() {
    let i = decode(0xfa01, 0xf082, ArmFeatures::CORTEX_M4F); // sxtah r0, r1, r2
    assert_eq!(i.op, Op::SXTH);
    assert_eq!((i.rd, i.rn, i.rm), (0, 1, 2));
}

#[test]
fn long_branches_match() {
    // Far targets (outside the stream, some wrapping below zero) exercise the high offset bits.
    let bad = run(LONG_BRANCH_VECTORS, ArmFeatures::CORTEX_M4F);
    assert!(bad.is_empty(), "{}", bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn every_operation_is_covered_by_the_vectors() {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    for (_, bytes, _) in VECTORS {
        let hw1 = u16::from_le_bytes([bytes[0], bytes[1]]);
        let hw2 = if bytes.len() > 2 { u16::from_le_bytes([bytes[2], bytes[3]]) } else { 0 };
        seen.insert(decode(hw1, hw2, ArmFeatures::CORTEX_M4F).op);
    }
    let missing: Vec<usize> = (0..mcs_core::arm::thumb::OP_COUNT)
        .filter(|&n| {
            // SAFETY: `Op` is `repr(u8)` with dense discriminants 0..OP_COUNT.
            let op: Op = unsafe { std::mem::transmute(n as u8) };
            op != Op::UNDEF && !seen.contains(&op)
        })
        .collect();
    let names: Vec<&str> = missing.iter().map(|&n| mcs_core::arm::thumb::OP_NAMES[n]).collect();
    assert!(missing.is_empty(), "ops without a vector: {:?}", names);
}
