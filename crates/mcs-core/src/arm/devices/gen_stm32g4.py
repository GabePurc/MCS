#!/usr/bin/env python3
"""Generates stm32g4_gen.rs (register templates, vectors, package pins, alternate functions).

Usage: gen_stm32g4.py <dir> [out.rs]
<dir> holds the vendor files (download once; the generated Rust file is checked in):
  stm32g431xx.h stm32g474xx.h   https://github.com/STMicroelectronics/cmsis-device-g4 (Include/)
  g431kt.xml g474rt.xml         STM32_open_pin_data mcu/STM32G431K(6-8-B)Tx.xml, STM32G474R(B-C-E)Tx.xml
  gpio43.xml gpio47.xml         STM32_open_pin_data mcu/IP/GPIO-STM32G43x|47x_gpio_v1_0_Modes.xml
Register offsets, bit masks and descriptions come from the CMSIS device header; reset values are
transcribed from RM0440 (the header does not carry them).
"""
import os
import re
import sys
import xml.etree.ElementTree as ET

D = sys.argv[1]
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "stm32g4_gen.rs")
HDR = open(os.path.join(D, "stm32g474xx.h")).read()

# (template, struct, bit prefix, [registers kept, in order])
TEMPLATES = [
    ("RCC", "RCC", "RCC", "CR ICSCR CFGR PLLCFGR CIER CIFR CICR AHB1RSTR AHB2RSTR AHB3RSTR APB1RSTR1 APB1RSTR2 APB2RSTR AHB1ENR AHB2ENR AHB3ENR APB1ENR1 APB1ENR2 APB2ENR CCIPR BDCR CSR CRRCR CCIPR2".split()),
    ("FLASH", "FLASH", "FLASH", "ACR KEYR OPTKEYR SR CR".split()),
    ("PWR", "PWR", "PWR", "CR1 CR2 CR3 CR4 SR1 SR2 SCR CR5".split()),
    ("SYSCFG", "SYSCFG", "SYSCFG", "MEMRMP CFGR1 EXTICR1 EXTICR2 EXTICR3 EXTICR4 SCSR CFGR2 SWPR SKR".split()),
    ("EXTI", "EXTI", "EXTI", "IMR1 EMR1 RTSR1 FTSR1 SWIER1 PR1".split()),
    ("GPIO", "GPIO", "GPIO", "MODER OTYPER OSPEEDR PUPDR IDR ODR BSRR LCKR AFRL AFRH BRR".split()),
    ("USART", "USART", "USART", "CR1 CR2 CR3 BRR GTPR RTOR RQR ISR ICR RDR TDR PRESC".split()),
    ("TIM_GP", "TIM", "TIM", "CR1 CR2 SMCR DIER SR EGR CCMR1 CCMR2 CCER CNT PSC ARR CCR1 CCR2 CCR3 CCR4".split()),
    ("TIM_BASIC", "TIM", "TIM", "CR1 CR2 DIER SR EGR CNT PSC ARR".split()),
]

# Reset values (RM0440 register descriptions); everything else resets to 0.
RESET = {
    "RCC": {"CR": 0x500, "ICSCR": 0x40000000, "PLLCFGR": 0x1000, "AHB1ENR": 0x100, "AHB2ENR": 0, "CSR": 0x0C000000},
    "FLASH": {"ACR": 0x40601, "CR": 0xC0000000, "SR": 0},
    "PWR": {"CR1": 0x200, "CR3": 0x8000, "CR5": 0x100},
    "SYSCFG": {"CFGR1": 0x7C000001},
    "EXTI": {"IMR1": 0xFF020000},
    "GPIO": {},
    "USART": {"ISR": 0xC0},
    "TIM_GP": {"ARR": 0xFFFF},
    "TIM_BASIC": {"ARR": 0xFFFF},
}
ACCESS = {"IDR": "r", "ISR": "r", "SR": None, "BSRR": "w", "BRR": "w", "ICR": "w", "RQR": "w", "EGR": "w", "CICR": "w"}


def rust_str(s):
    s = re.sub(r"\s+", " ", s).strip()
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def struct_members(name):
    end = HDR.index("} %s_TypeDef;" % name)
    start = HDR.rindex("typedef struct", 0, end)
    body = HDR[start:end]
    out = []
    for m in re.finditer(r"^\s*(?:__IO|__I|__O)?\s*u?int(?:8|16|32)_t\s+(\w+)(\[\d+\])?\s*;\s*/\*!<(.*?)\*/", body, re.M):
        nm, arr, cm = m.group(1), m.group(2), m.group(3)
        if nm.startswith("RESERVED"):
            continue
        off = re.search(r"Address offset:\s*(0x[0-9A-Fa-f]+)", cm)
        if not off:
            continue
        desc = cm.split(",")[0].strip()
        out.append((nm, arr, int(off.group(1), 16), desc, cm))
    return out


def bit_defs(prefix, reg):
    """[(name, mask, desc)] of PREFIX_REG_* fields that have a _Msk definition."""
    pat = re.compile(r"^#define\s+%s_%s_(\w+?)_Msk\s+\(0x([0-9A-Fa-f]+)U?L?\s*<<" % (prefix, reg), re.M)
    out, seen = [], set()
    for m in pat.finditer(HDR):
        bit, mask = m.group(1), int(m.group(2), 16)
        if bit in seen or "_" in bit and bit.split("_")[-1].isdigit() and False:
            continue
        seen.add(bit)
        dm = re.search(r"^#define\s+%s_%s_%s\s+\S+\s*/\*!<(.*?)\*/" % (prefix, reg, bit), HDR, re.M)
        desc = dm.group(1).strip() if dm else ""
        # Shift the mask by its position.
        pm = re.search(r"%s_%s_%s_Pos\s+\((\d+)U?\)" % (prefix, reg, bit), HDR)
        pos = int(pm.group(1)) if pm else 0
        out.append((bit, mask << pos, desc))
    return out


def template_rs(tname, struct, prefix, keep):
    members = {}
    for nm, arr, off, desc, cm in struct_members(struct):
        if arr:  # GPIO AFR[2]
            if nm == "AFR":
                members["AFRL"] = (off, "GPIO alternate function low register")
                members["AFRH"] = (off + 4, "GPIO alternate function high register")
            else:
                for i in range(int(arr[1:-1])):
                    members["%s%d" % (nm, i + 1)] = (off + 4 * i, "%s %d" % (desc, i + 1))
            continue
        members[nm] = (off, desc)
    lines = ["pub const %s: &[RegDef] = &[" % tname]
    for r in keep:
        off, desc = members[r]
        reset = RESET.get(tname, {}).get(r, 0)
        acc = ACCESS.get(r, "rw")
        if acc is None:
            acc = "rw"
        bits_reg = {"AFRL": "AFRL", "AFRH": "AFRH"}.get(r, r)
        bits = bit_defs(prefix, bits_reg)
        # Generic "AFSEL" fields of AFRL/AFRH are named AFSEL0..7 / AFSEL8..15 in the header.
        bl = ", ".join('BitDef { name: %s, mask: 0x%08x, desc: %s }' % (rust_str(b), m, rust_str(d)) for b, m, d in bits)
        lines.append("    RegDef { name: %s, off: 0x%x, reset: 0x%08x, access: %s, desc: %s, bits: &[%s] }," % (rust_str(r), off, reset, rust_str(acc), rust_str(desc), bl))
    lines.append("];")
    return "\n".join(lines)


def vectors(hdr_path):
    t = open(hdr_path).read()
    body = re.search(r"typedef enum\s*\{(.*?)\}\s*IRQn_Type;", t, re.S).group(1)
    out = []
    for m in re.finditer(r"^\s*(\w+)_IRQn\s*=\s*(-?\d+)\s*,?\s*(?:/\*!<(.*?)\*/)?", body, re.M):
        n, v, c = m.group(1), int(m.group(2)), (m.group(3) or "").strip()
        if v >= 0:
            out.append((v, n, c))
    return out


NS = {"m": "http://dummy.com"}


def pins(xml_path):
    root = ET.parse(xml_path).getroot()
    out = []
    for p in root.findall("m:Pin", NS):
        name, pos, typ = p.get("Name"), p.get("Position"), p.get("Type")
        if not pos.isdigit():
            raise SystemExit("non numeric pin position %s" % pos)
        sigs = [s.get("Name") for s in p.findall("m:Signal", NS) if s.get("Name") != "GPIO"]
        m = re.match(r"P([A-G])(\d+)", name)
        if typ == "Power":
            kind = 2 if name.startswith("VSS") else 1
            gpio = -1
        elif name.startswith("VREF"):
            kind, gpio = 3, -1
        elif m:
            kind, gpio = 0, (ord(m.group(1)) - 65) * 16 + int(m.group(2))
        else:
            kind, gpio = 0, -1
        out.append((int(pos), name, kind, gpio, sigs))
    out.sort()
    return out


KEEP_SIG = re.compile(r"^(USART[1-3]|UART[45]|LPUART1)_(TX|RX)$|^TIM[2-4]_CH[1-4]$")


def afs(gpio_xml, pin_list):
    root = ET.parse(gpio_xml).getroot()
    wanted = {name.split("-")[0] for _, name, kind, g, _ in pin_list if g >= 0}
    out = []
    for gp in root.findall("m:GPIO_Pin", NS):
        nm = gp.get("Name")
        if nm not in wanted:
            continue
        idx = (ord(nm[1]) - 65) * 16 + int(nm[2:])
        for ps in gp.findall("m:PinSignal", NS):
            sn = ps.get("Name")
            if not KEEP_SIG.match(sn):
                continue
            val = ps.find("m:SpecificParameter/m:PossibleValue", NS).text
            af = int(re.match(r"GPIO_AF(\d+)_", val).group(1))
            out.append((idx, af, sn))
    out.sort()
    return out


def dev_rs(tag, hdr, xml, gpio_xml):
    pl = pins(os.path.join(D, xml))
    L = ["pub const %s_PINS: &[PinDef] = &[" % tag]
    for pos, name, kind, gpio, sigs in pl:
        L.append("    PinDef { number: %d, name: %s, kind: %d, gpio: %d, functions: &[%s] }," % (pos, rust_str(name), kind, gpio, ", ".join(rust_str(s) for s in sigs)))
    L.append("];")
    L.append("pub const %s_AF: &[(u8, u8, &str)] = &[%s];" % (tag, ", ".join("(%d, %d, %s)" % (i, a, rust_str(s)) for i, a, s in afs(os.path.join(D, gpio_xml), pl))))
    L.append("pub const %s_VECTORS: &[(u16, &str, &str)] = &[%s];" % (tag, ", ".join("(%d, %s, %s)" % (v, rust_str(n), rust_str(c)) for v, n, c in vectors(os.path.join(D, hdr)))))
    return "\n".join(L)


def bases():
    defs = dict(re.findall(r"^#define\s+(\w+_BASE)\s+\(?([^\n/]*?)\)?\s*(?:/\*.*)?$", HDR, re.M))

    def ev(name):
        e = defs[name]
        e = re.sub(r"(0x[0-9A-Fa-f]+)U?L?", r"\1", e)
        return eval(re.sub(r"[A-Z0-9_]+_BASE", lambda m: str(ev(m.group(0))), e))

    names = "RCC FLASH_R PWR SYSCFG EXTI GPIOA GPIOB GPIOC GPIOD GPIOE GPIOF GPIOG USART1 USART2 USART3 UART4 UART5 LPUART1 TIM2 TIM3 TIM4 TIM6 TIM7".split()
    return "pub const BASES: &[(&str, u32)] = &[%s];" % ", ".join('("%s", 0x%08x)' % (n.replace("FLASH_R", "FLASH"), ev(n + "_BASE")) for n in names)


def main():
    out = [
        "// @generated by gen_stm32g4.py from the STM32G4 CMSIS device header (register layout, bit fields, IRQ names)",
        "// and STM32_open_pin_data (package pins, alternate functions); do not edit by hand.",
        "#![allow(clippy::all)]",
        "",
        "pub struct BitDef {",
        "    pub name: &'static str,",
        "    pub mask: u32,",
        "    pub desc: &'static str,",
        "}",
        "",
        "pub struct RegDef {",
        "    pub name: &'static str,",
        "    pub off: u32,",
        "    pub reset: u32,",
        "    pub access: &'static str,",
        "    pub desc: &'static str,",
        "    pub bits: &'static [BitDef],",
        "}",
        "",
        "pub struct PinDef {",
        "    pub number: u8,",
        "    pub name: &'static str,",
        "    /// 0 I/O, 1 supply, 2 ground, 3 reference.",
        "    pub kind: u8,",
        "    /// GPIO index (port * 16 + bit), -1 when the pin is not a GPIO.",
        "    pub gpio: i16,",
        "    pub functions: &'static [&'static str],",
        "}",
        "",
    ]
    for t in TEMPLATES:
        out.append(template_rs(*t))
        out.append("")
    out.append(bases())
    out.append("")
    out.append(dev_rs("G431KB", "stm32g431xx.h", "g431kt.xml", "gpio43.xml"))
    out.append("")
    out.append(dev_rs("G474RE", "stm32g474xx.h", "g474rt.xml", "gpio47.xml"))
    out.append("")
    open(OUT, "w").write("\n".join(out))


main()
