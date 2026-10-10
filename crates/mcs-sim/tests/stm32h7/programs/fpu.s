.include "common.inc"
.fpu fpv5-d16
vectors reset=reset

@ Double-precision FPU (FPv5-D16): results stored as raw doubles at 0x20000100 (8 bytes each):
@   +0  1.5 * 2.25                    +8  1.0 / 3.0
@   +16 sqrt(2.0)                     +24 fma(0.1, 10.0, -1.0) (fused, single rounding)
@   +32 (double)(int)  123.9 -> 123   +40 double(-7) + 0.5
@   +48 1.0 + 2^-53 stays 1.0 (round to nearest even), +56 vcmp 3.0 > 2.0 flags (as word)
func reset
  li r0, CPACR
  ldr r1, [r0]
  orr r1, r1, #(0xf << 20)                 @ CP10 + CP11 full access
  str r1, [r0]
  dsb
  isb
  li r4, 0x20000100
  adr r5, consts
  vldr d0, [r5, #0]                        @ 1.5
  vldr d1, [r5, #8]                        @ 2.25
  vmul.f64 d2, d0, d1
  vstr d2, [r4, #0]
  vldr d3, [r5, #16]                       @ 1.0
  vldr d4, [r5, #24]                       @ 3.0
  vdiv.f64 d5, d3, d4
  vstr d5, [r4, #8]
  vldr d6, [r5, #32]                       @ 2.0
  vsqrt.f64 d7, d6
  vstr d7, [r4, #16]
  vldr d8, [r5, #40]                       @ 0.1
  vldr d9, [r5, #48]                       @ 10.0
  vneg.f64 d10, d3                         @ accumulator = -1.0
  vfma.f64 d10, d8, d9                     @ d10 = d10 + d8 * d9 (fused: 0.1 * 10 - 1 = 2^-54-ish, not 0)
  vstr d10, [r4, #24]
  vldr d11, [r5, #56]                      @ 123.9
  vcvt.s32.f64 s0, d11                     @ truncate -> 123
  vcvt.f64.s32 d12, s0
  vstr d12, [r4, #32]
  movs r1, #7
  rsbs r1, r1, #0
  vmov s2, r1
  vcvt.f64.s32 d13, s2                     @ -7.0
  vldr d14, [r5, #64]                      @ 0.5
  vadd.f64 d13, d13, d14
  vstr d13, [r4, #40]
  vldr d15, [r5, #72]                      @ 2^-53
  vadd.f64 d15, d3, d15                    @ 1.0 + 2^-53 -> 1.0
  vstr d15, [r4, #48]
  vcmp.f64 d4, d6                          @ 3.0 vs 2.0
  vmrs APSR_nzcv, fpscr
  mrs r1, apsr
  str r1, [r4, #56]
.global done
done:
  bkpt #0
  b done

.balign 8
consts:
  .double 1.5
  .double 2.25
  .double 1.0
  .double 3.0
  .double 2.0
  .double 0.1
  .double 10.0
  .double 123.9
  .double 0.5
  .double 1.1102230246251565e-16

default_handler
