#!/usr/bin/env python3
"""Builds the ESP32-C3 test programs into linked ELF files (elf/*.elf, checked in so the tests need no toolchain).

Usage: python3 gen_programs.py [name ...]

Needs a Rust toolchain with the `riscv32imc-unknown-none-elf` target. Every `programs/*.s` is assembled through
`core::arch::global_asm!` (prefixed with `programs/common.inc`); every `programs/*.rs` is compiled as a `no_std` crate
with DWARF line info (`-C debuginfo=2`). All are linked by rust-lld with `link.ld`: .text at the IROM window
(0x4200_0000), .rodata in the DROM window (0x3C01_0000), .data / .bss in DRAM (0x3FC8_0000).
"""
import glob
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
PROGS = os.path.join(HERE, "programs")
OUT = os.path.join(HERE, "elf")
HEADER = '#![no_std]\n#![no_main]\n#[panic_handler] fn p(_: &core::panic::PanicInfo) -> ! { loop {} }\n'


def build(name, ext):
    with tempfile.TemporaryDirectory() as tmp:
        rs = os.path.join(tmp, name + ".rs")
        if ext == "s":
            # Bundled examples (ex_*) are self-contained and keep the assembler's default compressed instructions.
            with open(os.path.join(PROGS, "common.inc")) as f:
                common = "" if name.startswith("ex_") else f.read()
            with open(os.path.join(PROGS, name + ".s")) as f:
                body = f.read()
            src = HEADER + 'core::arch::global_asm!(r#"\n' + common + "\n" + body + '\n"#);\n'
            debug = "0"
        else:
            with open(os.path.join(PROGS, name + ".rs")) as f:
                src = f.read()
            debug = "2"
        with open(rs, "w") as f:
            f.write(src)
        out = os.path.join(OUT, name + ".elf")
        # --remap-path-prefix keeps the DWARF file names independent of the temp directory.
        r = subprocess.run(
            ["rustc", "--target", "riscv32imc-unknown-none-elf", "--crate-type", "bin", "-C", "panic=abort", "-C", "opt-level=1", "-C", f"debuginfo={debug}",
             "-C", "codegen-units=1", "-C", "relocation-model=static", f"-Clink-arg=-T{os.path.join(HERE, 'link.ld')}", "-Clink-arg=-znorelro",
             f"--remap-path-prefix={tmp}=tests/esp32c3/programs", rs, "-o", out],
            capture_output=True, text=True,
        )
        if r.returncode:
            sys.exit(f"{name} failed to build:\n{r.stderr[:4000]}")
        print(name, os.path.getsize(out), "bytes")


def main():
    os.makedirs(OUT, exist_ok=True)
    progs = sorted((os.path.basename(p)[:-2], "s") for p in glob.glob(os.path.join(PROGS, "*.s"))) + sorted((os.path.basename(p)[:-3], "rs") for p in glob.glob(os.path.join(PROGS, "*.rs")))
    wanted = set(sys.argv[1:])
    for name, ext in progs:
        if not wanted or name in wanted:
            build(name, ext)


if __name__ == "__main__":
    main()
