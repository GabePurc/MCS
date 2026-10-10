#!/usr/bin/env python3
"""Generates the llvm-objdump reference vectors for the RISC-V decoder/disassembler tests.

Usage:
  python3 gen_vectors.py vectors.rs            # checked-in sample (a few thousand lines)
  python3 gen_vectors.py --full out.txt        # exhaustive local run (every 16-bit encoding + a large
                                               # random 32-bit set); used with RISCV_FULL=out.txt
  python3 gen_vectors.py --csr-rs csr_names.rs # CSR number -> name table as printed by objdump
                                               # (checked in as crates/mcs-core/src/riscv/csr_names.rs)

Needs a Rust toolchain with the `riscv32imc-unknown-none-elf` target (patterns are assembled with
`.insn` through rustc's `global_asm!`) and the `llvm-objdump` shipped with that toolchain
(`rustup component add llvm-tools` if missing). The generated file is checked in, so the tests do
not need any toolchain.

Every pattern sits in its own 8-byte slot (`.balign 8`) so one undecodable pattern cannot desync the
following ones; a pattern is only kept when objdump reports it at exactly its slot address with the
expected length.
"""
import os
import random
import re
import subprocess
import sys
import tempfile

BASE = 0x42000000
SYSROOT = subprocess.check_output(["rustc", "--print", "sysroot"]).decode().strip()
HOST = subprocess.check_output(["rustc", "-vV"]).decode().split("host: ")[1].split()[0]
OBJDUMP = os.path.join(SYSROOT, "lib", "rustlib", HOST, "bin", "llvm-objdump")

rng = random.Random(0x5eed)

# ---------------------------------------------------------------------------------------------
# 32-bit encoders
# ---------------------------------------------------------------------------------------------


def r_type(f7, rs2, rs1, f3, rd, op):
    return (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op


def i_type(imm, rs1, f3, rd, op):
    return ((imm & 0xFFF) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op


def s_type(imm, rs2, rs1, f3, op):
    imm &= 0xFFF
    return ((imm >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((imm & 0x1F) << 7) | op


def b_type(imm, rs2, rs1, f3):
    imm &= 0x1FFF
    return (
        ((imm >> 12) << 31)
        | (((imm >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | (((imm >> 1) & 0xF) << 8)
        | (((imm >> 11) & 1) << 7)
        | 0x63
    )


def u_type(imm20, rd, op):
    return ((imm20 & 0xFFFFF) << 12) | (rd << 7) | op


def j_type(imm, rd):
    imm &= 0x1FFFFF
    return (
        ((imm >> 20) << 31)
        | (((imm >> 1) & 0x3FF) << 21)
        | (((imm >> 11) & 1) << 20)
        | (((imm >> 12) & 0xFF) << 12)
        | (rd << 7)
        | 0x6F
    )


def reg():
    return rng.choice([0, 1, 2, 5, 8, 10, 15, 31, rng.randrange(32), rng.randrange(32)])


def imm12():
    return rng.choice([0, 1, -1, 2047, -2048, 0x7F, -16, 8, rng.randrange(-2048, 2048), rng.randrange(-2048, 2048)])


def gen32(n):
    """Valid 32-bit instructions covering every opcode with random / edge operands."""
    out = []
    alu_r = [(0, 0), (0x20, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0x20, 5), (0, 6), (0, 7)]
    mext = [(1, f3) for f3 in range(8)]
    for _ in range(n):
        k = rng.randrange(24)
        if k == 0:
            f7, f3 = rng.choice(alu_r + mext)
            out.append(r_type(f7, reg(), reg(), f3, reg(), 0x33))
        elif k == 1:
            f3 = rng.choice([0, 2, 3, 4, 6, 7])
            out.append(i_type(imm12(), reg(), f3, reg(), 0x13))
        elif k == 2:
            f3, f7 = rng.choice([(1, 0), (5, 0), (5, 0x20)])
            out.append(r_type(f7, rng.randrange(32), reg(), f3, reg(), 0x13))
        elif k == 3:
            out.append(i_type(imm12(), reg(), rng.choice([0, 1, 2, 4, 5]), reg(), 0x03))
        elif k == 4:
            out.append(s_type(imm12(), reg(), reg(), rng.choice([0, 1, 2]), 0x23))
        elif k == 5:
            off = rng.choice([0, 2, 4, -2, -4, 4094, -4096, rng.randrange(-2048, 2048) * 2])
            out.append(b_type(off, reg(), reg(), rng.choice([0, 1, 4, 5, 6, 7])))
        elif k == 6:
            off = rng.choice([0, 2, 4, -2, -4, 0xFFFFE, -0x100000, rng.randrange(-0x80000, 0x80000) * 2])
            out.append(j_type(off, reg()))
        elif k == 7:
            out.append(i_type(imm12(), reg(), 0, reg(), 0x67))
        elif k == 8:
            out.append(u_type(rng.choice([0, 1, 0xFFFFF, 0x80000, 0x12345, rng.randrange(1 << 20)]), reg(), 0x37))
        elif k == 9:
            out.append(u_type(rng.choice([0, 1, 0xFFFFF, 0x80000, 0x12345, rng.randrange(1 << 20)]), reg(), 0x17))
        elif k == 10:
            out.append(i_type((rng.randrange(16) << 4) | rng.randrange(16) | (rng.choice([0, 0, 8]) << 8), 0, 0, 0, 0x0F))
        elif k == 11:
            out.append(rng.choice([0x0000100F, 0x8330000F, 0x0100000F, 0x0FF0000F, 0x0220000F, 0x0110000F]))
        elif k == 12:
            out.append(rng.choice([0x00000073, 0x00100073, 0x30200073, 0x10500073, 0x10200073, 0xC0001073]))
        else:
            # Zicsr: named, unnamed and read-only CSR numbers
            csr = rng.choice([0x300, 0x301, 0x304, 0x305, 0x340, 0x341, 0x342, 0x343, 0x344, 0xB00, 0xB02, 0xB80, 0xB82,
                              0xC00, 0xC01, 0xC02, 0xC80, 0xC82, 0xF11, 0xF12, 0xF13, 0xF14, 0x7C0, 0x800, 0x001,
                              0x002, 0x003, 0x100, 0x7B0, 0xFFF, rng.randrange(4096), rng.randrange(4096)])
            f3 = rng.choice([1, 2, 3, 5, 6, 7])
            rs1 = reg() if f3 < 4 else rng.choice([0, 1, 5, 31, rng.randrange(32)])
            out.append(i_type(csr, rs1, f3, reg(), 0x73))
    return out


def gen32_systematic():
    """Every operation with a few fixed register / immediate choices (aliases included)."""
    out = []
    alu_r = [(0, 0), (0x20, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5), (0x20, 5), (0, 6), (0, 7)] + [(1, f) for f in range(8)]
    for f7, f3 in alu_r:
        for rd, rs1, rs2 in [(10, 11, 12), (5, 0, 6), (7, 8, 0), (0, 0, 0), (31, 31, 31), (1, 0, 5), (3, 4, 4)]:
            out.append(r_type(f7, rs2, rs1, f3, rd, 0x33))
    for f3 in (0, 2, 3, 4, 6, 7):
        for rd, rs1, imm in [(10, 11, 5), (10, 0, 5), (10, 11, 0), (0, 0, 0), (5, 6, -1), (5, 6, 1), (2, 2, -16), (0, 10, 0x40), (0, 3, 1)]:
            out.append(i_type(imm, rs1, f3, rd, 0x13))
    for f3, f7 in [(1, 0), (5, 0), (5, 0x20)]:
        for sh in (0, 1, 15, 31):
            out.append(r_type(f7, sh, 9, f3, 18, 0x13))
    for f3 in (0, 1, 2, 4, 5):
        for imm in (0, -4, 2047, -2048):
            out.append(i_type(imm, 9, f3, 18, 0x03))
    for f3 in (0, 1, 2):
        for imm in (0, -4, 2047, -2048):
            out.append(s_type(imm, 18, 9, f3, 0x23))
    for f3 in (0, 1, 4, 5, 6, 7):
        for rs1, rs2 in [(10, 11), (10, 0), (0, 10), (0, 0)]:
            for off in (8, -8, 4094):
                out.append(b_type(off, rs2, rs1, f3))
    for rd in (0, 1, 5):
        for off in (0, 2, -2, 0xFFFFE):
            out.append(j_type(off, rd))
    for rd, rs1, imm in [(0, 1, 0), (0, 5, 0), (1, 5, 0), (1, 5, 8), (0, 5, 8), (6, 5, 0), (6, 5, -4)]:
        out.append(i_type(imm, rs1, 0, rd, 0x67))
    for rd in (0, 5, 31):
        out.append(u_type(0x12345, rd, 0x37))
        out.append(u_type(0xFFFFF, rd, 0x17))
    for csr in (0x300, 0x305, 0xC00, 0xC80, 0x7C0):
        for f3 in (1, 2, 3, 5, 6, 7):
            for rd, rs1 in [(10, 11), (0, 11), (10, 0), (0, 0), (10, 31)]:
                out.append(i_type(csr, rs1, f3, rd, 0x73))
    out += [0x0000100F, 0x8330000F, 0x0100000F, 0x0FF0000F, 0x0000000F, 0x0220000F, 0x0110000F, 0x00000073, 0x00100073, 0x30200073, 0x10500073]
    return out


def gen32_illegal(n):
    """Random words in the 32-bit encoding space (mostly undefined)."""
    out = []
    for _ in range(n):
        w = rng.getrandbits(32) | 3
        if (w >> 2) & 7 == 7:
            w &= ~(1 << 2)
        out.append(w)
    return out


def gen16_all():
    return [h for h in range(0x10000) if h & 3 != 3]


def gen16_sample(n):
    """Compressed encodings: random per quadrant/funct3 plus structured edge cases."""
    out = []
    for q in range(3):
        for f3 in range(8):
            for _ in range(n):
                out.append((f3 << 13) | (rng.getrandbits(11) << 2) | q)
    # the encodings a compiler emits, with fixed registers
    for rd in (1, 2, 8, 10, 15, 31):
        out.append(0x4001 | (rd << 7))  # c.li rd, 0
        out.append(0x0001 | (rd << 7) | (5 << 2))  # c.addi
        out.append(0x8002 | (rd << 7))  # c.jr rd
        out.append(0x9002 | (rd << 7))  # c.jalr rd
        out.append(0x8006 | (rd << 7))  # c.mv rd, ra
        out.append(0x9006 | (rd << 7))  # c.add rd, ra
    out += [0x0001, 0x9002, 0x8082, 0x0000, 0x6105, 0x1141, 0xA001, 0x4501]
    return out


# ---------------------------------------------------------------------------------------------
# assembling / disassembling
# ---------------------------------------------------------------------------------------------


def disassemble(patterns):
    """patterns: list of (length, value). Returns list of (address, bytes, text) kept entries."""
    lines = [".section .text.start,\"ax\"", ".globl _start", "_start:"]
    for ln, v in patterns:
        lines.append(f".balign 8\n .insn {ln}, 0x{v:0{ln * 2}x}")
    asm = "\n".join(lines)
    src = '#![no_std]\n#![no_main]\n#[panic_handler] fn p(_: &core::panic::PanicInfo) -> ! { loop {} }\n'
    src += 'core::arch::global_asm!(r#"\n' + asm + '\n"#);\n'
    with tempfile.TemporaryDirectory() as tmp:
        rs = os.path.join(tmp, "t.rs")
        elf = os.path.join(tmp, "t.elf")
        with open(rs, "w") as f:
            f.write(src)
        subprocess.check_call(
            ["rustc", "--target", "riscv32imc-unknown-none-elf", "-C", "panic=abort", f"-Clink-arg=-Ttext=0x{BASE:x}", "-O", rs, "-o", elf]
        )
        dump = subprocess.check_output([OBJDUMP, "-d", elf]).decode()
    seen = {}
    for line in dump.splitlines():
        m = re.match(r"^\s*([0-9a-f]{8}):\s+([0-9a-f]+)\s*\t(.*)$", line)
        if m:
            seen[int(m.group(1), 16)] = (m.group(2), m.group(3).strip())
    out = []
    addr = BASE
    for ln, v in patterns:
        if addr in seen:
            raw, text = seen[addr]
            if len(raw) == ln * 2 and int(raw, 16) == v:
                text = re.sub(r"\s*<[^>]*>", "", text)
                text = " ".join(text.split()) or "<unknown>"
                out.append((addr, list(v.to_bytes(ln, "little")), text))
        addr = (addr + ln + 7) & ~7
    return out


def write_rs(path, entries):
    with open(path, "w") as f:
        f.write("// @generated by gen_vectors.py from llvm-objdump (rustc llvm-tools, riscv32imc); do not edit by hand.\n")
        f.write("/// (address, little-endian bytes, expected text); `<unknown>` marks encodings objdump rejects.\n")
        f.write("pub const VECTORS: &[(u32, &[u8], &str)] = &[\n")
        for a, b, t in entries:
            bs = ", ".join(f"0x{x:02x}" for x in b)
            f.write(f'    (0x{a:x}, &[{bs}], "{t}"),\n')
        f.write("];\n")


def chunked(patterns, size=20000):
    out = []
    for i in range(0, len(patterns), size):
        out += disassemble(patterns[i : i + size])
    return out


def main():
    args = sys.argv[1:]
    if args and args[0] == "--csr-rs":
        pats = [(4, i_type(c, 0, 2, 10, 0x73)) for c in range(4096)]
        names = []
        for a, b, t in chunked(pats):
            w = int.from_bytes(bytes(b), "little")
            m = re.match(r"^csrr a0, ([a-z][a-z0-9_]*)$", t)
            if m:
                names.append((w >> 20, m.group(1)))
            else:
                m = re.match(r"^rd([a-z]+) a0$", t)
                if m:
                    names.append((w >> 20, m.group(1)))
        with open(args[1], "w") as f:
            f.write("// @generated by crates/mcs-core/tests/riscv_decode/gen_vectors.py --csr-rs (llvm-objdump CSR names); do not edit.\n")
            f.write("/// `(csr number, name)` sorted by number; the names `llvm-objdump` prints for the standard CSRs.\n")
            f.write("pub const CSR_NAMES: &[(u16, &str)] = &[\n")
            for n, nm in sorted(names):
                f.write(f'    (0x{n:03x}, "{nm}"),\n')
            f.write("];\n")
        print(len(names), "csr names")
        return
    if args and args[0] == "--full":
        pats = [(2, h) for h in gen16_all()]
        pats += [(4, w) for w in gen32(120000)]
        pats += [(4, w) for w in gen32_illegal(60000)]
        ent = chunked(pats)
        with open(args[1], "w") as f:
            for a, b, t in ent:
                f.write(f"{a:x} {''.join(f'{x:02x}' for x in b)} {t}\n")
        print(len(ent), "entries")
        return
    pats = [(2, h) for h in gen16_sample(60)]
    pats += [(4, w) for w in gen32_systematic()]
    pats += [(4, w) for w in gen32(1700)]
    pats += [(4, w) for w in gen32_illegal(150)]
    ent = chunked(pats)
    seen = set()
    uniq = []
    for e in ent:
        key = bytes(e[1])
        if key in seen:
            continue
        seen.add(key)
        uniq.append(e)
    write_rs(args[0], uniq)
    print(len(uniq), "vectors")


if __name__ == "__main__":
    main()
