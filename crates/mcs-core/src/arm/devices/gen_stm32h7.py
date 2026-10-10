#!/usr/bin/env python3
"""Generates stm32h7_gen.rs (register templates, vectors, package pins, alternate functions, clock-enable
positions) for the STM32H743.

Usage: gen_stm32h7.py <dir> [out.rs]
<dir> holds the vendor files (download once; the generated Rust file is checked in):
  stm32h743xx.h   https://github.com/STMicroelectronics/cmsis_device_h7 (Include/)
  h743ii.xml      STM32_open_pin_data mcu/STM32H743IITx.xml  (LQFP176)
  h743zi.xml      STM32_open_pin_data mcu/STM32H743ZITx.xml  (LQFP144)
  gpio_h7.xml     STM32_open_pin_data mcu/IP/GPIO-STM32H747_gpio_v1_0_Modes.xml
  h743.svd        modm-io/cmsis-svd-stm32 stm32h7/STM32H743.svd (ST's SVD; the CMSIS header carries no reset values)
Register offsets, bit masks and descriptions come from the CMSIS device header, reset values from the SVD
(a few corrected from the reference manual, see RESET_OVERRIDE), clock-enable bit positions from the header's
RCC_xxxENR_yyyEN_Pos definitions.
"""
import os
import re
import sys
import xml.etree.ElementTree as ET

D = sys.argv[1]
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "stm32h7_gen.rs")
HDR = open(os.path.join(D, "stm32h743xx.h")).read()
SVD = ET.parse(os.path.join(D, "h743.svd")).getroot()

# (template, struct, bit prefix, svd peripheral, [registers kept, in order])
TEMPLATES = [
    ("RCC", "RCC", "RCC", "RCC", "CR HSICFGR CRRCR CSICFGR CFGR D1CFGR D2CFGR D3CFGR PLLCKSELR PLLCFGR PLL1DIVR PLL1FRACR PLL2DIVR PLL2FRACR PLL3DIVR PLL3FRACR D1CCIPR D2CCIP1R D2CCIP2R D3CCIPR CIER CIFR CICR BDCR CSR AHB3RSTR AHB1RSTR AHB2RSTR AHB4RSTR APB3RSTR APB1LRSTR APB1HRSTR APB2RSTR APB4RSTR GCR D3AMR RSR AHB3ENR AHB1ENR AHB2ENR AHB4ENR APB3ENR APB1LENR APB1HENR APB2ENR APB4ENR".split()),
    ("FLASH", "FLASH", "FLASH", "Flash", "ACR KEYR1 OPTKEYR CR1 SR1 CCR1 OPTCR OPTSR_CUR OPTSR_PRG OPTCCR KEYR2 CR2 SR2 CCR2".split()),
    ("PWR", "PWR", "PWR", "PWR", "CR1 CSR1 CR2 CR3 CPUCR D3CR WKUPCR WKUPFR WKUPEPR".split()),
    ("SYSCFG", "SYSCFG", "SYSCFG", "SYSCFG", "PMCR EXTICR1 EXTICR2 EXTICR3 EXTICR4 CFGR CCCSR CCVR CCCR PWRCR PKGR".split()),
    ("EXTI", "EXTI", "EXTI", "EXTI", "RTSR1 FTSR1 SWIER1 D3PMR1 D3PCR1L D3PCR1H IMR1 EMR1 PR1".split()),
    ("GPIO", "GPIO", "GPIO", None, "MODER OTYPER OSPEEDR PUPDR IDR ODR BSRR LCKR AFRL AFRH".split()),
    ("USART", "USART", "USART", "USART1", "CR1 CR2 CR3 BRR GTPR RTOR RQR ISR ICR RDR TDR PRESC".split()),
    ("TIM_GP", "TIM", "TIM", "TIM2", "CR1 CR2 SMCR DIER SR EGR CCMR1 CCMR2 CCER CNT PSC ARR CCR1 CCR2 CCR3 CCR4".split()),
    ("TIM_BASIC", "TIM", "TIM", "TIM6", "CR1 CR2 DIER SR EGR CNT PSC ARR".split()),
]

# Header register name -> SVD register name when they differ.
SVD_NAME = {"EXTI": {"IMR1": "CPUIMR1", "EMR1": "CPUEMR1", "PR1": "CPUPR1"}}
# Header bit-definition register for registers that share definitions (bank 1 / bank 2).
BITS_REG = {"CR1": "CR", "CR2": "CR", "SR1": "SR", "SR2": "SR", "CCR1": "CCR", "CCR2": "CCR", "AFRL": "AFRL", "AFRH": "AFRH"}
# Reset values the SVD lacks or gets wrong (RM0433 register descriptions).
# h743.svd (modm-io/cmsis-svd-stm32) was cross-checked against cmsis-svd-data's STM32H743x.svd
# (data/STMicro/STM32H743x.svd): every RCC / PWR / SYSCFG / EXTI / GPIO / USART / TIM reset value is
# identical; the only differences are FLASH_ACR (0x37 vs 0x600, the latter is a G4-style value that is
# not valid on the H7: RM0433 gives LATENCY = 7, WRHIGHFREQ = 3 = 0x37), FLASH_CR2 (0x31 vs 0:
# LOCK + PSIZE = 3 per RM0433, so CR1 gets 0x31 too although the SVD lacks it) and RCC_HSICFGR (only
# in one file; 0x40000000 = HSITRIM default). OPTCR.OPTLOCK = 1 and the GPIO port A / B pull and mode
# values below are RM0433 (same as the G4 ports; the SVD copies port A values to all ports).
RESET_OVERRIDE = {
    "FLASH": {"CR1": 0x31, "OPTCR": 0x1},  # LOCK + PSIZE = 3; OPTLOCK
    "GPIO": {"MODER": 0xFFFFFFFF},  # ports A and B are corrected in Rust (JTAG/SWD pins)
    "TIM_GP": {"ARR": 0xFFFF},  # TIM2/TIM5 (32-bit) are corrected in Rust
    "TIM_BASIC": {"ARR": 0xFFFF},
}
ACCESS = {"IDR": "r", "ISR": "r", "BSRR": "w", "ICR": "w", "RQR": "w", "EGR": "w", "CICR": "w", "CCR1": "w", "CCR2": "w", "CSR1": "r", "PKGR": "r"}


def rust_str(s):
    s = re.sub(r"\s+", " ", s).strip()
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def svd_resets(periph):
    out = {}
    if periph is None:
        return out
    for p in SVD.iter("peripheral"):
        if p.findtext("name") == periph:
            for r in p.iter("register"):
                v = r.findtext("resetValue")
                if v:
                    out[r.findtext("name")] = int(v, 16)
    return out


def struct_members(name):
    end = HDR.index("}%s_TypeDef;" % name) if ("}%s_TypeDef;" % name) in HDR else HDR.index("} %s_TypeDef;" % name)
    start = HDR.rindex("typedef struct", 0, end)
    body = HDR[start:end]
    out = {}
    for m in re.finditer(r"^\s*(?:__IO|__I|__O)?\s*u?int(?:8|16|32)_t\s+(\w+)(\[\d+\])?\s*;\s*/\*!<(.*?)\*/", body, re.M):
        nm, arr, cm = m.group(1), m.group(2), m.group(3)
        if nm.startswith("RESERVED"):
            continue
        off = re.search(r"Address offset:\s*(0x[0-9A-Fa-f]+)", cm)
        if not off:
            continue
        off = int(off.group(1), 16)
        desc = cm.split(",")[0].strip()
        if arr:
            if nm == "AFR":
                out["AFRL"] = (off, "GPIO alternate function low register")
                out["AFRH"] = (off + 4, "GPIO alternate function high register")
            else:
                for i in range(int(arr[1:-1])):
                    out["%s%d" % (nm, i + 1)] = (off + 4 * i, "%s %d" % (desc, i + 1))
        else:
            out[nm] = (off, desc)
    return out


def bit_defs(prefix, reg):
    """[(name, mask, desc)] of PREFIX_REG_* fields that have a _Msk definition (named sub-values such as
    HPRE_DIV2 are dropped)."""
    pat = re.compile(r"^#define\s+%s_%s_(\w+?)_Msk\s+\(0x([0-9A-Fa-f]+)U?L?\s*<<\s*\w+_Pos\)" % (prefix, reg), re.M)
    found = [(m.group(1), int(m.group(2), 16)) for m in pat.finditer(HDR)]
    names = {n for n, _ in found}
    out = []
    for bit, mask in found:
        if any(bit.startswith(o + "_") for o in names):
            continue
        dm = re.search(r"^#define\s+%s_%s_%s\s+\S+\s*/\*!<(.*?)\*/" % (prefix, reg, bit), HDR, re.M)
        pm = re.search(r"%s_%s_%s_Pos\s+\((\d+)U?\)" % (prefix, reg, bit), HDR)
        out.append((bit, mask << (int(pm.group(1)) if pm else 0), dm.group(1).strip() if dm else ""))
    return out


def template_rs(tname, struct, prefix, svd_periph, keep):
    members = struct_members(struct)
    svd = svd_resets(svd_periph)
    lines = ["pub const %s: &[RegDef] = &[" % tname]
    for r in keep:
        off, desc = members[r]
        reset = RESET_OVERRIDE.get(tname, {}).get(r)
        if reset is None:
            reset = svd.get(SVD_NAME.get(tname, {}).get(r, r), 0)
        acc = ACCESS.get(r, "rw")
        bits = bit_defs(prefix, BITS_REG.get(r, r))
        bl = ", ".join("BitDef { name: %s, mask: 0x%08x, desc: %s }" % (rust_str(b), m, rust_str(d)) for b, m, d in bits)
        lines.append("    RegDef { name: %s, off: 0x%x, reset: 0x%08x, access: %s, desc: %s, bits: &[%s] }," % (rust_str(r), off, reset, rust_str(acc), rust_str(desc), bl))
    lines.append("];")
    return "\n".join(lines)


def vectors():
    body = re.search(r"typedef enum\s*\{(.*?)\}\s*IRQn_Type;", HDR, re.S).group(1)
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
        m = re.match(r"P([A-K])(\d+)", name)
        if name.startswith("VREF"):
            kind, gpio = 3, -1
        elif typ == "Power":
            kind = 2 if name.startswith("VSS") else 1
            gpio = -1
        elif m:
            kind, gpio = 0, (ord(m.group(1)) - 65) * 16 + int(m.group(2))
        else:
            kind, gpio = 0, -1
        out.append((int(pos), name, kind, gpio, sigs))
    out.sort()
    return out


KEEP_SIG = re.compile(r"^(USART[1236]|UART[4578]|LPUART1)_(TX|RX)$|^TIM[2-5]_CH[1-4]$")


def afs(gpio_xml, pin_list):
    root = ET.parse(gpio_xml).getroot()
    wanted = {name.split("-")[0] for _, name, kind, g, _ in pin_list if g >= 0}
    out = set()
    for gp in root.findall("m:GPIO_Pin", NS):
        nm = gp.get("Name")
        if nm not in wanted:
            continue
        idx = (ord(nm[1]) - 65) * 16 + int(re.match(r"P.(\d+)", nm).group(1))
        for ps in gp.findall("m:PinSignal", NS):
            sn = ps.get("Name")
            if not KEEP_SIG.match(sn):
                continue
            val = ps.find("m:SpecificParameter/m:PossibleValue", NS).text
            out.add((idx, int(re.match(r"GPIO_AF(\d+)_", val).group(1)), sn))
    return sorted(out)


def dev_rs(tag, xml):
    pl = pins(os.path.join(D, xml))
    # A GPIO may only appear on one package pin (PA0 / PA0_C style pairs would break the pin array).
    seen = {}
    for pos, name, kind, gpio, _ in pl:
        if gpio >= 0:
            assert gpio not in seen, "GPIO %d on pins %s and %s" % (gpio, seen[gpio], name)
            seen[gpio] = name
    L = ["pub const %s_PINS: &[PinDef] = &[" % tag]
    for pos, name, kind, gpio, sigs in pl:
        L.append("    PinDef { number: %d, name: %s, kind: %d, gpio: %d, functions: &[%s] }," % (pos, rust_str(name), kind, gpio, ", ".join(rust_str(s) for s in sigs)))
    L.append("];")
    L.append("pub const %s_AF: &[(u8, u8, &str)] = &[%s];" % (tag, ", ".join("(%d, %d, %s)" % (i, a, rust_str(s)) for i, a, s in afs(os.path.join(D, "gpio_h7.xml"), pl))))
    return "\n".join(L)


def bases():
    defs = dict(re.findall(r"^#define\s+(\w+_BASE)\s+\(?([^\n/]*?)\)?\s*(?:/\*.*)?$", HDR, re.M))

    def ev(name):
        e = re.sub(r"(0x[0-9A-Fa-f]+)U?L?", r"\1", defs[name])
        return eval(re.sub(r"[A-Z0-9_]+_BASE", lambda m: str(ev(m.group(0))), e))

    names = "RCC FLASH_R PWR SYSCFG EXTI GPIOA GPIOB GPIOC GPIOD GPIOE GPIOF GPIOG GPIOH GPIOI GPIOJ GPIOK USART1 USART2 USART3 UART4 UART5 USART6 UART7 UART8 LPUART1 TIM2 TIM3 TIM4 TIM5 TIM6 TIM7".split()
    return "pub const BASES: &[(&str, u32)] = &[%s];" % ", ".join('("%s", 0x%08x)' % (n.replace("FLASH_R", "FLASH"), ev(n + "_BASE")) for n in names)


# RCC enable registers in the order the simulator indexes them (sys.enr): AHB3, AHB1, AHB2, AHB4, APB3, APB1L, APB1H, APB2, APB4.
ENR = "AHB3ENR AHB1ENR AHB2ENR AHB4ENR APB3ENR APB1LENR APB1HENR APB2ENR APB4ENR".split()


def enables():
    items = []
    spec = {"GPIO%s" % c: ("AHB4ENR", "GPIO%sEN" % c) for c in "ABCDEFGHIJK"}
    for t in "TIM2 TIM3 TIM4 TIM5 TIM6 TIM7 USART2 USART3 UART4 UART5 UART7 UART8".split():
        spec[t] = ("APB1LENR", t + "EN")
    spec["USART1"] = ("APB2ENR", "USART1EN")
    spec["USART6"] = ("APB2ENR", "USART6EN")
    spec["LPUART1"] = ("APB4ENR", "LPUART1EN")
    spec["SYSCFG"] = ("APB4ENR", "SYSCFGEN")
    for inst, (reg, field) in spec.items():
        m = re.search(r"^#define\s+RCC_%s_%s_Pos\s+\((\d+)U\)" % (reg, field), HDR, re.M)
        assert m, (reg, field)
        items.append('("%s", %d, %s)' % (inst, ENR.index(reg), m.group(1)))
    return "pub const ENABLE: &[(&str, u8, u8)] = &[%s];" % ", ".join(items)


def main():
    out = [
        "// @generated by gen_stm32h7.py from the STM32H743 CMSIS device header (register layout, bit fields, IRQ names,",
        "// clock-enable bits), ST's SVD (reset values) and STM32_open_pin_data (package pins, alternate functions);",
        "// do not edit by hand.",
        "#![allow(clippy::all)]",
        "",
        "pub use super::gen_types::{BitDef, PinDef, RegDef};",
        "",
    ]
    for t in TEMPLATES:
        out.append(template_rs(*t))
        out.append("")
    out.append(bases())
    out.append("")
    out.append("/// Clock-enable bit of each modelled peripheral: (instance, RCC enable register index, bit). Register indices:")
    out.append("/// 0 AHB3, 1 AHB1, 2 AHB2, 3 AHB4, 4 APB3, 5 APB1L, 6 APB1H, 7 APB2, 8 APB4.")
    out.append(enables())
    out.append("")
    out.append("pub const VECTORS: &[(u16, &str, &str)] = &[%s];" % ", ".join("(%d, %s, %s)" % (v, rust_str(n), rust_str(c)) for v, n, c in vectors()))
    out.append("")
    out.append(dev_rs("H743II", "h743ii.xml"))
    out.append("")
    out.append(dev_rs("H743ZI", "h743zi.xml"))
    out.append("")
    open(OUT, "w").write("\n".join(out))


main()
