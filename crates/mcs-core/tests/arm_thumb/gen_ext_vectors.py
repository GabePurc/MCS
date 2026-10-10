#!/usr/bin/env python3
"""Generates vectors_ext.rs: DSP-extension and floating-point reference encodings.

Every instruction below is assembled with Apple clang and disassembled with llvm-objdump; the
(address, bytes, text) triples are written to vectors_ext.rs, which is checked in so the tests do
not need clang.

  M4F_VECTORS   thumbv7em-none-eabihf, -mcpu=cortex-m4        (DSP + FPv4-SP)
  M7_VECTORS    thumbv7em-none-eabihf, -mcpu=cortex-m7 -mfpu=fpv5-d16   (FPv5-D16: double
                precision, VSEL, VMAXNM, VRINT*, VCVTA/N/P/M, vmov.32 scalar)

Usage: python3 gen_ext_vectors.py [out.rs]   (needs the Xcode command line tools)
"""
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OBJDUMP = subprocess.check_output(["xcrun", "--find", "llvm-objdump"]).decode().strip()

CORE = ["r0", "r1", "r2", "r3", "r7", "r8", "r12", "lr"]
TRIPLES = [("r0", "r1", "r2"), ("r8", "r9", "r10"), ("r12", "lr", "r3"), ("r4", "r4", "r4"), ("r7", "r0", "r11")]
S3 = [(0, 1, 2), (31, 30, 29), (1, 2, 3), (16, 17, 18), (5, 10, 20), (30, 1, 15), (7, 8, 9), (2, 1, 0)]
D3 = [(0, 1, 2), (15, 14, 13), (1, 2, 3), (8, 9, 10), (5, 10, 12), (2, 1, 0)]
S2 = [(0, 1), (31, 30), (1, 2), (16, 17), (5, 10), (30, 1), (2, 1)]
D2 = [(0, 1), (15, 14), (1, 2), (8, 9), (12, 5)]


def vfp_expand(imm8, dp):
    """VFPExpandImm as a Python float (the exponent field is NOT(b):b..b:cd)."""
    sign = -1.0 if imm8 & 0x80 else 1.0
    b = (imm8 >> 6) & 1
    cd = (imm8 >> 4) & 3
    exponent = cd + 1 if b == 0 else cd - 3
    return sign * (16 + (imm8 & 15)) / 16.0 * (2.0 ** exponent)


def dsp_lines():
    L = []
    kinds = ["add16", "asx", "sax", "sub16", "add8", "sub8"]
    for p in ["s", "q", "sh", "u", "uq", "uh"]:
        for k in kinds:
            for t in TRIPLES[:3]:
                L.append("%s%s %s, %s, %s" % (p, k, *t))
    for t in TRIPLES:
        L.append("sel %s, %s, %s" % t)
        L.append("usad8 %s, %s, %s" % t)
        L.append("usada8 %s, %s, %s, r5" % t)
    for sat in [1, 8, 16]:
        L.append("ssat16 r0, #%d, r1" % sat)
        L.append("ssat16 r12, #%d, lr" % sat)
    for sat in [0, 7, 15]:
        L.append("usat16 r0, #%d, r1" % sat)
    for sh in [0, 1, 8, 31]:
        L.append("pkhbt r0, r1, r2" + (", lsl #%d" % sh if sh else ""))
    for sh in [1, 8, 16, 31, 32]:
        L.append("pkhtb r3, r4, r5, asr #%d" % sh)
    L.append("pkhbt r8, r9, r10, lsl #12")
    for rot in [0, 8, 16, 24]:
        r = ", ror #%d" % rot if rot else ""
        L.append("sxtb16 r0, r1" + r)
        L.append("uxtb16 r8, r9" + r)
        L.append("sxtab16 r0, r1, r2" + r)
        L.append("uxtab16 r3, r4, r5" + r)
    for t in TRIPLES[:3]:
        for xy in ["bb", "bt", "tb", "tt"]:
            L.append("smul%s %s, %s, %s" % (xy, *t))
            L.append("smla%s %s, %s, %s, r5" % (xy, *t))
            L.append("smlal%s %s, %s, %s, %s" % (xy, "r0", "r1", t[1], t[2]))
        for y in ["b", "t"]:
            L.append("smulw%s %s, %s, %s" % (y, *t))
            L.append("smlaw%s %s, %s, %s, r6" % (y, *t))
        for x in ["", "x"]:
            L.append("smuad%s %s, %s, %s" % (x, *t))
            L.append("smusd%s %s, %s, %s" % (x, *t))
            L.append("smlad%s %s, %s, %s, r5" % (x, *t))
            L.append("smlsd%s %s, %s, %s, r5" % (x, *t))
            L.append("smlald%s r0, r1, %s, %s" % (x, t[1], t[2]))
            L.append("smlsld%s r8, r9, %s, %s" % (x, t[1], t[2]))
        for r in ["", "r"]:
            L.append("smmul%s %s, %s, %s" % (r, *t))
            L.append("smmla%s %s, %s, %s, r5" % (r, *t))
            L.append("smmls%s %s, %s, %s, r5" % (r, *t))
        L.append("umaal r0, r1, %s, %s" % (t[1], t[2]))
    L.append("umaal r8, r9, r10, r12")
    L.append("qadd16 r0, r1, r2")
    return L


def fp_lines(dp):
    f = ".f64" if dp else ".f32"
    R = lambda n: ("d%d" if dp else "s%d") % n
    T3 = D3 if dp else S3
    T2 = D2 if dp else S2
    L = []
    for op in ["vadd", "vsub", "vmul", "vnmul", "vdiv", "vmla", "vmls", "vnmla", "vnmls", "vfma", "vfms", "vfnma", "vfnms"]:
        for a, b, c in T3:
            L.append("%s%s %s, %s, %s" % (op, f, R(a), R(b), R(c)))
    for op in ["vabs", "vneg", "vsqrt", "vmov"]:
        for a, b in T2:
            L.append("%s%s %s, %s" % (op, f, R(a), R(b)))
    for a, b in T2:
        L.append("vcmp%s %s, %s" % (f, R(a), R(b)))
        L.append("vcmpe%s %s, %s" % (f, R(a), R(b)))
    for a, _ in T2[:3]:
        L.append("vcmp%s %s, #0" % (f, R(a)))
        L.append("vcmpe%s %s, #0" % (f, R(a)))
    for imm8 in list(range(0, 256, 5)) + [255, 127, 128, 112, 0x70, 0x80]:
        v = vfp_expand(imm8, dp)
        L.append("vmov%s %s, #%r" % (f, R(imm8 % 16 if dp else (imm8 * 7) % 32), v))
    # memory
    for off in [0, 4, 8, 1020, -4, -8, -1020]:
        for base in ["r0", "r7", "r12", "sp", "pc"] if off >= 0 else ["r1", "sp"]:
            for n in ([0, 9, 15] if dp else [0, 17, 31]):
                if off:
                    L.append("vldr %s, [%s, #%d]" % (R(n), base, off))
                else:
                    L.append("vldr %s, [%s]" % (R(n), base))
        for n in ([1, 14] if dp else [1, 30]):
            if base != "pc":
                pass
            L.append("vstr %s, [r2, #%d]" % (R(n), off) if off else "vstr %s, [r2]" % R(n))
    counts = [1, 2, 4, 8, 16] if dp else [1, 2, 4, 16, 32]
    for cnt in counts:
        for first in ([0, 8, 16 - cnt] if dp else [0, 16, 32 - cnt]):
            if first < 0 or first + cnt > (16 if dp else 32):
                continue
            lst = "{%s-%s}" % (R(first), R(first + cnt - 1)) if cnt > 1 else "{%s}" % R(first)
            L.append("vldmia r0, %s" % lst)
            L.append("vldmia r1!, %s" % lst)
            L.append("vldmdb r2!, %s" % lst)
            L.append("vstmia r3, %s" % lst)
            L.append("vstmia r4!, %s" % lst)
            L.append("vstmdb r5!, %s" % lst)
            L.append("vpush %s" % lst)
            L.append("vpop %s" % lst)
    # core <-> fp
    for c in CORE:
        L.append("vmov %s, s%d" % (c, (CORE.index(c) * 5) % 32))
        L.append("vmov s%d, %s" % ((CORE.index(c) * 7 + 1) % 32, c))
    for a, b in [("r0", "r1"), ("r8", "r9"), ("r12", "lr"), ("r3", "r2")]:
        for s in [0, 1, 30, 7]:
            if s > 30:
                continue
            L.append("vmov s%d, s%d, %s, %s" % (s, s + 1, a, b))
            L.append("vmov %s, %s, s%d, s%d" % (a, b, s, s + 1))
    if dp:
        for a, b in [("r0", "r1"), ("r8", "r9"), ("r12", "lr"), ("r3", "r2")]:
            for d in [0, 1, 15, 7]:
                L.append("vmov d%d, %s, %s" % (d, a, b))
                L.append("vmov %s, %s, d%d" % (a, b, d))
        for c in ["r0", "r5", "r12", "lr"]:
            for d in [0, 7, 15]:
                for lane in [0, 1]:
                    L.append("vmov.32 d%d[%d], %s" % (d, lane, c))
                    L.append("vmov.32 %s, d%d[%d]" % (c, d, lane))
    for c in ["r0", "r1", "r12", "lr"]:
        L.append("vmrs %s, fpscr" % c)
        L.append("vmsr fpscr, %s" % c)
    L.append("vmrs APSR_nzcv, fpscr")
    for reg in ["fpsid", "mvfr0", "mvfr1"] + (["mvfr2"] if dp else []):
        L.append("vmrs r2, %s" % reg)
    # conversions
    for t in ["s32", "u32"]:
        for a, b in [(0, 1), (31, 30), (16, 5)]:
            sd = "s%d" % a
            src = ("d%d" % (b % 16)) if dp else "s%d" % b
            L.append("vcvt.%s%s %s, %s" % (t, f, sd, src))
            L.append("vcvtr.%s%s %s, %s" % (t, f, sd, src))
            dst = ("d%d" % (a % 16)) if dp else "s%d" % a
            L.append("vcvt%s.%s %s, s%d" % (f, t, dst, b))
    for t in ["s16", "u16", "s32", "u32"]:
        bits = 16 if t.endswith("16") else 32
        for fb in sorted(set([1, 3, bits // 2, bits])):
            L.append("vcvt.%s%s %s, %s, #%d" % (t, f, R(3), R(3), fb))
            L.append("vcvt%s.%s %s, %s, #%d" % (f, t, R(14), R(14), fb))
    if not dp:
        for a, b in [(0, 1), (31, 30), (7, 16)]:
            L.append("vcvtb.f32.f16 s%d, s%d" % (a, b))
            L.append("vcvtt.f32.f16 s%d, s%d" % (a, b))
            L.append("vcvtb.f16.f32 s%d, s%d" % (a, b))
            L.append("vcvtt.f16.f32 s%d, s%d" % (a, b))
    # IT blocks
    L.append("it eq")
    L.append("vaddeq%s %s, %s, %s" % (f, R(0), R(1), R(2)))
    L.append("ite ne")
    L.append("vmovne%s %s, %s" % (f, R(3), R(4)))
    L.append("vldreq %s, [r0, #4]" % R(5))
    L.append("itt mi")
    L.append("vpushmi {%s}" % R(8))
    L.append("vmrsmi r0, fpscr")
    return L


def m7_lines():
    L = fp_lines(True)
    for cc in ["eq", "vs", "ge", "gt"]:
        for dp in [False, True]:
            f = ".f64" if dp else ".f32"
            r = (lambda n: "d%d" % n) if dp else (lambda n: "s%d" % n)
            for a, b, c in [(0, 1, 2), (15 if dp else 31, 14 if dp else 30, 13 if dp else 29)]:
                L.append("vsel%s%s %s, %s, %s" % (cc, f, r(a), r(b), r(c)))
    for dp in [False, True]:
        f = ".f64" if dp else ".f32"
        r = (lambda n: "d%d" % n) if dp else (lambda n: "s%d" % n)
        for a, b, c in [(0, 1, 2), (15 if dp else 31, 14 if dp else 30, 13 if dp else 29), (5, 6, 7)]:
            L.append("vmaxnm%s %s, %s, %s" % (f, r(a), r(b), r(c)))
            L.append("vminnm%s %s, %s, %s" % (f, r(a), r(b), r(c)))
        for m in "azrxnpm":
            for a, b in [(0, 1), (15 if dp else 31, 14 if dp else 30), (5, 9)]:
                if m in "azrx" or True:
                    L.append("vrint%s%s %s, %s" % (m, f, r(a), r(b)))
        for m in "anpm":
            for t in ["s32", "u32"]:
                for a, b in [(0, 1), (31, 7), (16, 5)]:
                    src = ("d%d" % (b % 16)) if dp else "s%d" % b
                    L.append("vcvt%s.%s%s s%d, %s" % (m, t, f, a, src))
    L.append("vcvt.f64.f32 d0, s1")
    L.append("vcvt.f64.f32 d15, s31")
    L.append("vcvt.f32.f64 s0, d1")
    L.append("vcvt.f32.f64 s31, d15")
    for a, b in [(0, 1), (15, 31), (7, 16)]:
        L.append("vcvtb.f64.f16 d%d, s%d" % (a, b))
        L.append("vcvtt.f64.f16 d%d, s%d" % (a, b))
        L.append("vcvtb.f16.f64 s%d, d%d" % (b, a))
        L.append("vcvtt.f16.f64 s%d, d%d" % (b, a))
    return L


def assemble(lines, flags, mcpu):
    with tempfile.TemporaryDirectory() as tmp:
        src = os.path.join(tmp, "a.s")
        obj = os.path.join(tmp, "a.o")
        with open(src, "w") as f:
            f.write(".syntax unified\n.thumb\n" + "\n".join(lines) + "\n")
        subprocess.check_call(["clang"] + flags + ["-c", src, "-o", obj])
        out = subprocess.check_output(
            [OBJDUMP, "-d", "--triple=thumbv7em-none-eabihf", "--mcpu=" + mcpu, obj]
        ).decode()
    vecs = []
    for line in out.splitlines():
        m = re.match(r"^\s*([0-9a-f]+):\s+((?:[0-9a-f]{4} ?)+)\s*\t(.*)$", line)
        if not m:
            continue
        addr = int(m.group(1), 16)
        hws = m.group(2).split()
        data = []
        for h in hws:
            v = int(h, 16)
            data += [v & 0xFF, v >> 8]
        vecs.append((addr, data, m.group(3).strip()))
    return vecs


def rust_vecs(name, vecs):
    out = ["pub const %s: &[(u32, &[u8], &str)] = &[" % name]
    for addr, data, text in vecs:
        t = text.replace("\\", "\\\\").replace('"', '\\"').replace("\t", "\\t")
        out.append("    (0x%x, &[%s], \"%s\")," % (addr, ", ".join("0x%02x" % b for b in data), t))
    out.append("];")
    return "\n".join(out)


def main():
    out_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "vectors_ext.rs")
    m4 = assemble(dsp_lines() + fp_lines(False), ["--target=thumbv7em-none-eabihf", "-mcpu=cortex-m4"], "cortex-m4")
    m7 = assemble(m7_lines(), ["--target=thumbv7em-none-eabihf", "-mcpu=cortex-m7", "-mfpu=fpv5-d16"], "cortex-m7")
    with open(out_path, "w") as f:
        f.write("// @generated by gen_ext_vectors.py from llvm-objdump (Apple clang, thumbv7em); do not edit by hand.\n")
        f.write("/// DSP + FPv4-SP encodings (Cortex-M4F): one contiguous stream starting at address 0.\n")
        f.write(rust_vecs("M4F_VECTORS", m4) + "\n\n")
        f.write("/// FPv5-D16 encodings (Cortex-M7): double precision, VSEL, VMAXNM, VRINT*, VCVTA/N/P/M.\n")
        f.write(rust_vecs("M7_VECTORS", m7) + "\n")
    print("M4F:", len(m4), "M7:", len(m7))


main()
