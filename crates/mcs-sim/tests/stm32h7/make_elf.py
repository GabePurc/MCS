#!/usr/bin/env python3
"""Builds a tiny linked ARM ELF (ET_EXEC, one PT_LOAD at 0x0800_0000) from programs/blink.s without a
linker: Apple clang assembles the source (DWARF 4 line table), this script lays .text out at the
flash base, relocates symbols and the line table by hand, and writes the result.

Usage: python3 make_elf.py [out.elf]   (default: crates/mcs-formats/tests/data/stm32h743_blink.elf)
"""
import os
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "..", "mcs-formats", "tests", "data", "stm32h743_blink.elf")
BASE = 0x08000000


def read_elf(path):
    d = open(path, "rb").read()
    shoff, = struct.unpack_from("<I", d, 32)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", d, 46)
    secs = []
    for i in range(shnum):
        name, typ, flags, addr, off, size, link, info, align, entsize = struct.unpack_from("<IIIIIIIIII", d, shoff + i * shentsize)
        secs.append(dict(name=name, type=typ, flags=flags, addr=addr, off=off, size=size, link=link, info=info, entsize=entsize))
    shstr = secs[shstrndx]
    for s in secs:
        end = d.index(b"\0", shstr["off"] + s["name"])
        s["sname"] = d[shstr["off"] + s["name"]:end].decode()
        s["data"] = d[s["off"]:s["off"] + s["size"]]
    return d, secs


def main():
    with tempfile.TemporaryDirectory() as tmp:
        obj = os.path.join(tmp, "blink.o")
        subprocess.check_call(["clang", "--target=thumbv7em-none-eabihf", "-mcpu=cortex-m7", "-mfpu=fpv5-d16", "-g", "-gdwarf-4", "-I", os.path.join(HERE, "programs"), "-c", os.path.join(HERE, "programs", "blink.s"), "-o", obj],
                              cwd=os.path.join(HERE, "programs"))
        _, secs = read_elf(obj)
    by = {s["sname"]: i for i, s in enumerate(secs)}
    text = secs[by[".text"]]["data"]
    text_idx = by[".text"]

    # Relocate the DWARF line table: DW_LNE_set_address operands carry R_ARM_ABS32 relocations.
    line = bytearray(secs[by[".debug_line"]]["data"])
    rel = secs[by[".rel.debug_line"]]["data"]
    symtab = secs[by[".symtab"]]
    syms = []
    for k in range(len(symtab["data"]) // 16):
        nm, val, size, info, other, shndx = struct.unpack_from("<IIIBBH", symtab["data"], k * 16)
        syms.append((nm, val, size, info, other, shndx))
    for k in range(len(rel) // 8):
        r_off, r_info = struct.unpack_from("<II", rel, k * 8)
        sym, typ = r_info >> 8, r_info & 0xff
        if typ == 2:  # R_ARM_ABS32
            inplace, = struct.unpack_from("<I", line, r_off)
            struct.pack_into("<I", line, r_off, (inplace + syms[sym][1] + BASE) & 0xffffffff)

    # New symbol table: the global/local function and object symbols defined in .text.
    strtab_old = secs[symtab["link"]]["data"]
    strtab = bytearray(b"\0")
    new_syms = [struct.pack("<IIIBBH", 0, 0, 0, 0, 0, 0)]
    first_global = 1
    entries = []
    for nm, val, size, info, other, shndx in syms[1:]:
        if shndx != text_idx or info & 0xf not in (0, 2):
            continue
        name = strtab_old[nm:strtab_old.index(b"\0", nm)]
        if not name or name.startswith(b"$") or name.startswith(b".L"):
            continue
        entries.append((info >> 4 != 0, name, val, size, info))
    entries.sort(key=lambda e: e[0])  # locals first
    for glob, name, val, size, info in entries:
        off = len(strtab)
        strtab += name + b"\0"
        new_syms.append(struct.pack("<IIIBBH", off, val + BASE, size, info, 0, 1))
        if not glob:
            first_global += 1
    symtab_data = b"".join(new_syms)

    # Layout: ehdr | phdr | .text | .symtab | .strtab | .debug_line | .shstrtab | shdrs
    shstr = bytearray(b"\0")
    def add(n):
        o = len(shstr)
        shstr.extend(n.encode() + b"\0")
        return o
    names = {n: add(n) for n in [".text", ".symtab", ".strtab", ".debug_line", ".shstrtab"]}
    text_off = 0x60
    pos = text_off + len(text)
    pos = (pos + 3) & ~3
    sym_off = pos
    pos += len(symtab_data)
    str_off = pos
    pos += len(strtab)
    line_off = pos
    pos += len(line)
    shstr_off = pos
    pos += len(shstr)
    pos = (pos + 3) & ~3
    sh_off = pos

    reset = next(v for g, n, v, s, i in entries if n == b"reset") + BASE
    ehdr = b"\x7fELF" + bytes([1, 1, 1, 0]) + bytes(8)
    ehdr += struct.pack("<HHIIIIIHHHHHH", 2, 40, 1, reset, 52, sh_off, 0x05000000, 52, 32, 1, 40, 6, 5)
    phdr = struct.pack("<IIIIIIII", 1, text_off, BASE, BASE, len(text), len(text), 5, 4)
    sh = lambda name, typ, flags, addr, off, size, link=0, info=0, align=1, ent=0: struct.pack("<IIIIIIIIII", name, typ, flags, addr, off, size, link, info, align, ent)
    shdrs = [
        sh(0, 0, 0, 0, 0, 0, 0, 0, 0),
        sh(names[".text"], 1, 6, BASE, text_off, len(text), align=4),
        sh(names[".symtab"], 2, 0, 0, sym_off, len(symtab_data), link=3, info=first_global, align=4, ent=16),
        sh(names[".strtab"], 3, 0, 0, str_off, len(strtab)),
        sh(names[".debug_line"], 1, 0, 0, line_off, len(line)),
        sh(names[".shstrtab"], 3, 0, 0, shstr_off, len(shstr)),
    ]
    out = bytearray(ehdr + phdr)
    out += bytes(text_off - len(out))
    out += text
    out += bytes(sym_off - len(out)) + symtab_data
    out += strtab
    out += line
    out += shstr
    out += bytes(sh_off - len(out))
    out += b"".join(shdrs)
    open(OUT, "wb").write(out)
    print("wrote", os.path.normpath(OUT), len(out), "bytes")


main()
