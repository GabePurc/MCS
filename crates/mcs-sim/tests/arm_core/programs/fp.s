.include "common.inc"
vectors reset=reset, usage=usage_h

@ FPv4-SP (Cortex-M4F): arithmetic, rounding modes, exceptions flags, NaN / denormal / flush-to-
@ zero handling, conversions, compares, moves, memory access and the CPACR access check.
@ Expected values are listed in tests/arm_core/main.rs (derived by hand from IEEE 754 and the
@ ARM ARM pseudocode).

@ Enables CP10 / CP11 (full access).
.macro fenable
  li r0, 0xe000ed88
  li r1, 0x00f00000
  str r1, [r0]
  dsb
  isb
.endm

@ Loads a 32-bit pattern into an S register.
.macro fli s, v
  li r0, \v
  vmov \s, r0
.endm

@ Output an S register / the exception flags (IDC, IXC, UFC, OFC, DZC, IOC) / the full FPSCR.
.macro outs s
  vmov r0, \s
  out r0
.endm
.macro outfl
  vmrs r0, fpscr
  and r0, r0, #0x9f
  out r0
.endm
.macro outfpscr
  vmrs r0, fpscr
  out r0
.endm

.macro fpscr_set v
  li r0, \v
  vmsr fpscr, r0
.endm

@ Outputs the NZCV nibble produced by a compare.
.macro outnzcv
  vmrs APSR_nzcv, fpscr
  mrs r0, apsr
  lsrs r0, r0, #28
  out r0
.endm

func reset
  setr12
  mrs r0, control
  out r0                         @ [0] CONTROL.FPCA clear before any FP instruction
  fenable
  @ ---- basic arithmetic ---------------------------------------------------------------------
  fli s1, 0x3fc00000             @ 1.5
  fli s2, 0x40100000             @ 2.25
  mrs r0, control
  out r0                         @ [1] FPCA set by the first FP instruction (FPCCR.ASPEN)
  fpscr_set 0
  vadd.f32 s0, s1, s2
  outs s0                        @ [2] 3.75
  outfl                          @ [3] 0
  vsub.f32 s0, s1, s2
  outs s0                        @ [4] -0.75
  vmul.f32 s0, s1, s2
  outs s0                        @ [5] 3.375
  vnmul.f32 s0, s1, s2
  outs s0                        @ [6] -3.375
  fli s3, 0x3f800000
  fli s4, 0x40400000
  vdiv.f32 s0, s3, s4
  outs s0                        @ [7] 1/3 = 0x3eaaaaab
  outfl                          @ [8] inexact
  fpscr_set 0
  fli s5, 0x40000000
  vsqrt.f32 s0, s5
  outs s0                        @ [9] sqrt(2)
  outfl                          @ [10] inexact
  fpscr_set 0
  fli s5, 0x40800000
  vsqrt.f32 s0, s5
  outs s0                        @ [11] 2.0
  outfl                          @ [12] exact
  fli s5, 0xc0000000
  vabs.f32 s0, s5
  outs s0                        @ [13] 2.0
  vneg.f32 s0, s1
  outs s0                        @ [14] -1.5
  @ ---- multiply-accumulate: non-fused versus fused ------------------------------------------
  fli s1, 0x3f800800             @ 1 + 2^-12
  fli s2, 0x3f800800
  fpscr_set 0
  fli s0, 0xbf801000             @ -(1 + 2^-11)
  vmla.f32 s0, s1, s2
  outs s0                        @ [15] rounded product cancels: +0
  outfl                          @ [16] inexact
  fpscr_set 0
  fli s0, 0xbf801000
  vfma.f32 s0, s1, s2
  outs s0                        @ [17] 2^-24 = 0x33800000
  outfl                          @ [18] exact
  fli s0, 0x3f801000             @ 1 + 2^-11
  vfms.f32 s0, s1, s2
  outs s0                        @ [19] -2^-24 = 0xb3800000
  fli s0, 0x3f801000
  vfnms.f32 s0, s1, s2
  outs s0                        @ [20] +2^-24
  fli s0, 0x3f801000
  vnmla.f32 s0, s1, s2
  outs s0                        @ [21] -2.0009765625 = 0xc0001000
  fli s0, 0x3f801000
  vnmls.f32 s0, s1, s2
  outs s0                        @ [22] 0
  fli s0, 0x3f801000
  vmls.f32 s0, s1, s2
  outs s0                        @ [23] 0
  @ ---- rounding modes -----------------------------------------------------------------------
  fli s3, 0x3f800000
  fli s4, 0x40400000
  fpscr_set 0x00000000           @ round to nearest
  vdiv.f32 s0, s3, s4
  outs s0                        @ [24] 0x3eaaaaab
  fpscr_set 0x00400000           @ toward +inf
  vdiv.f32 s0, s3, s4
  outs s0                        @ [25] 0x3eaaaaab
  fpscr_set 0x00800000           @ toward -inf
  vdiv.f32 s0, s3, s4
  outs s0                        @ [26] 0x3eaaaaaa
  fpscr_set 0x00c00000           @ toward zero
  vdiv.f32 s0, s3, s4
  outs s0                        @ [27] 0x3eaaaaaa
  fli s5, 0x3f800000
  fli s6, 0xbf800000
  fpscr_set 0
  vadd.f32 s0, s5, s6
  outs s0                        @ [28] +0
  fpscr_set 0x00800000
  vadd.f32 s0, s5, s6
  outs s0                        @ [29] -0 when rounding toward -inf
  fli s7, 0x7f7fffff
  fpscr_set 0
  vadd.f32 s0, s7, s7
  outs s0                        @ [30] +inf
  outfl                          @ [31] overflow + inexact
  fpscr_set 0x00c00000
  vadd.f32 s0, s7, s7
  outs s0                        @ [32] largest finite number
  fpscr_set 0x00400000
  fli s5, 0x3fc00000
  vmul.f32 s0, s5, s4            @ 1.5 * 3 = 4.5 exact
  outs s0                        @ [33] 0x40900000
  @ ---- float -> integer ---------------------------------------------------------------------
  fpscr_set 0
  fli s1, 0x402ccccd             @ 2.7
  vcvt.s32.f32 s0, s1
  outs s0                        @ [34] 2
  outfl                          @ [35] inexact
  fpscr_set 0
  vcvtr.s32.f32 s0, s1
  outs s0                        @ [36] 3 (round to nearest)
  fli s1, 0x40200000             @ 2.5
  vcvtr.s32.f32 s0, s1
  outs s0                        @ [37] 2 (ties to even)
  fli s1, 0x40600000             @ 3.5
  vcvtr.s32.f32 s0, s1
  outs s0                        @ [38] 4
  fpscr_set 0x00400000
  fli s1, 0x40066666             @ 2.1
  vcvtr.s32.f32 s0, s1
  outs s0                        @ [39] 3 (round toward +inf)
  fpscr_set 0
  fli s1, 0xc02ccccd             @ -2.7
  vcvt.s32.f32 s0, s1
  outs s0                        @ [40] -2
  fpscr_set 0
  fli s1, 0x4f800000             @ 2^32
  vcvt.s32.f32 s0, s1
  outs s0                        @ [41] saturates to 0x7fffffff
  outfl                          @ [42] invalid operation
  fpscr_set 0
  vcvt.u32.f32 s0, s1
  outs s0                        @ [43] 0xffffffff
  outfl                          @ [44] invalid operation
  fpscr_set 0
  fli s1, 0x4f000000             @ 2^31
  vcvt.u32.f32 s0, s1
  outs s0                        @ [45] 0x80000000
  outfl                          @ [46] exact
  fli s1, 0x7fc00000             @ NaN
  vcvt.s32.f32 s0, s1
  outs s0                        @ [47] 0
  outfl                          @ [48] invalid operation
  fpscr_set 0
  fli s1, 0xbf800000             @ -1.0
  vcvt.u32.f32 s0, s1
  outs s0                        @ [49] 0
  outfl                          @ [50] invalid operation
  fpscr_set 0
  fli s1, 0xbf000000             @ -0.5
  vcvt.u32.f32 s0, s1
  outs s0                        @ [51] 0
  outfl                          @ [52] inexact only
  @ ---- integer -> float, fixed point, half precision ----------------------------------------
  fpscr_set 0
  fli s1, 0xfffffffb
  vcvt.f32.s32 s0, s1
  outs s0                        @ [53] -5.0
  fli s1, 0xffffffff
  vcvt.f32.u32 s0, s1
  outs s0                        @ [54] 4294967296.0
  outfl                          @ [55] inexact
  fpscr_set 0
  fli s1, 0x01000001             @ 16777217
  vcvt.f32.s32 s0, s1
  outs s0                        @ [56] 16777216.0
  fpscr_set 0x00400000
  vcvt.f32.s32 s0, s1
  outs s0                        @ [57] 16777218.0
  fpscr_set 0
  fli s0, 0x40200000             @ 2.5
  vcvt.s16.f32 s0, s0, #4
  outs s0                        @ [58] 40
  fli s0, 0x40200000
  vcvt.u32.f32 s0, s0, #8
  outs s0                        @ [59] 640
  fpscr_set 0
  fli s0, 0x47c35000             @ 100000.0
  vcvt.s16.f32 s0, s0, #4
  outs s0                        @ [60] 0x7fff
  outfl                          @ [61] invalid operation
  fpscr_set 0
  fli s0, 0xc7c35000
  vcvt.s16.f32 s0, s0, #4
  outs s0                        @ [62] 0xffff8000
  fli s0, 0x180
  vcvt.f32.u32 s0, s0, #8
  outs s0                        @ [63] 1.5
  fli s0, 0xffff
  vcvt.f32.s16 s0, s0, #4
  outs s0                        @ [64] -0.0625
  fpscr_set 0
  fli s0, 0xdead0000
  fli s1, 0x3fc00000
  vcvtb.f16.f32 s0, s1
  outs s0                        @ [65] 0xdead3e00
  vcvtt.f16.f32 s0, s1
  outs s0                        @ [66] 0x3e003e00
  vcvtb.f32.f16 s2, s0
  outs s2                        @ [67] 1.5
  vcvtt.f32.f16 s3, s0
  outs s3                        @ [68] 1.5
  fli s1, 0x4788b800             @ 70000.0: overflows binary16
  vcvtb.f16.f32 s0, s1
  outs s0                        @ [69] 0x00007c00 (the top half was cleared by the previous VCVTT
                                 @      keeping 0x3e00 -> 0x3e007c00)
  outfl                          @ [70] overflow + inexact
  @ ---- compares -----------------------------------------------------------------------------
  fpscr_set 0
  fli s1, 0x3f800000             @ 1.0
  fli s2, 0x40000000             @ 2.0
  vcmp.f32 s1, s2
  outnzcv                        @ [71] less: N = 8
  vcmp.f32 s2, s1
  outnzcv                        @ [72] greater: C = 2
  vcmp.f32 s1, s1
  outnzcv                        @ [73] equal: Z C = 6
  vcmp.f32 s1, #0
  outnzcv                        @ [74] greater than zero: 2
  fli s3, 0x7fc00000
  vcmp.f32 s1, s3
  outnzcv                        @ [75] unordered: C V = 3
  outfl                          @ [76] quiet NaN: no exception for VCMP
  vcmpe.f32 s1, s3
  outnzcv                        @ [77] 3
  outfl                          @ [78] invalid operation for VCMPE
  fpscr_set 0
  fli s3, 0x7f800001             @ signalling NaN
  vcmp.f32 s3, s1
  outnzcv                        @ [79] 3
  outfl                          @ [80] invalid operation
  fpscr_set 0
  vcmp.f32 s1, s2
  vmrs APSR_nzcv, fpscr
  ite lt
  movlt r4, #1
  movge r4, #0
  out r4                         @ [81] 1 < 2 -> LT taken
  vcmp.f32 s2, s1
  vmrs APSR_nzcv, fpscr
  ite gt
  movgt r4, #1
  movle r4, #0
  out r4                         @ [82] 2 > 1 -> GT taken
  vcmp.f32 s2, s1
  outfpscr                       @ [83] FPSCR holds the flags: C = 0x20000000
  @ ---- NaN, infinity, denormals, flush to zero ----------------------------------------------
  fpscr_set 0
  fli s1, 0x7f800000
  vsub.f32 s0, s1, s1
  outs s0                        @ [84] default NaN 0x7fc00000
  outfl                          @ [85] invalid operation
  fpscr_set 0
  fli s2, 0
  vmul.f32 s0, s1, s2
  outs s0                        @ [86] 0x7fc00000
  outfl                          @ [87] invalid operation
  fpscr_set 0
  fli s1, 0x7fc01234
  fli s2, 0x3f800000
  vadd.f32 s0, s1, s2
  outs s0                        @ [88] payload kept: 0x7fc01234
  outfl                          @ [89] none
  fli s1, 0x7f800001
  vadd.f32 s0, s1, s2
  outs s0                        @ [90] quieted: 0x7fc00001
  outfl                          @ [91] invalid operation
  fpscr_set 0x02000000           @ DN: default NaN mode
  fli s1, 0x7fc01234
  vadd.f32 s0, s1, s2
  outs s0                        @ [92] 0x7fc00000
  fpscr_set 0
  fli s1, 0x00000001
  vadd.f32 s0, s1, s1
  outs s0                        @ [93] denormal arithmetic: 2
  outfl                          @ [94] exact
  fpscr_set 0x01000000           @ FZ
  vadd.f32 s0, s1, s1
  outs s0                        @ [95] flushed: 0
  outfl                          @ [96] input denormal: IDC = 0x80
  fpscr_set 0
  fli s1, 0x00800001
  fli s2, 0x3f000000
  vmul.f32 s0, s1, s2
  outs s0                        @ [97] tiny, inexact: 0x00400000
  outfl                          @ [98] underflow + inexact = 0x18
  fpscr_set 0
  fli s1, 0x00800000
  vmul.f32 s0, s1, s2
  outs s0                        @ [99] tiny, exact: 0x00400000
  outfl                          @ [100] none
  fpscr_set 0x01000000
  vmul.f32 s0, s1, s2
  outs s0                        @ [101] flush to zero: 0
  outfl                          @ [102] underflow = 0x08
  fpscr_set 0
  fli s1, 0x3f800000
  fli s2, 0
  vdiv.f32 s0, s1, s2
  outs s0                        @ [103] +inf
  outfl                          @ [104] divide by zero = 0x02
  fli s1, 0xbf800000
  vdiv.f32 s0, s1, s2
  outs s0                        @ [105] -inf
  fpscr_set 0
  vdiv.f32 s0, s2, s2
  outs s0                        @ [106] 0x7fc00000
  outfl                          @ [107] invalid operation
  fpscr_set 0
  vsqrt.f32 s0, s1
  outs s0                        @ [108] sqrt(-1): 0x7fc00000
  outfl                          @ [109] invalid operation
  fpscr_set 0
  fli s1, 0x80000000
  vsqrt.f32 s0, s1
  outs s0                        @ [110] sqrt(-0) = -0
  fli s1, 0x7fc00000
  vneg.f32 s0, s1
  outs s0                        @ [111] VNEG only flips the sign: 0xffc00000
  vabs.f32 s0, s0
  outs s0                        @ [112] 0x7fc00000
  outfl                          @ [113] none
  @ ---- moves and FPSCR ----------------------------------------------------------------------
  vmov.f32 s0, #1.0
  outs s0                        @ [114] 0x3f800000
  vmov.f32 s0, #-0.5
  outs s0                        @ [115] 0xbf000000
  vmov.f32 s0, #31.0
  outs s0                        @ [116] 0x41f80000
  vmov.f32 s0, #0.125
  vmov.f32 s5, s0
  outs s5                        @ [117] 0x3e000000
  li r1, 0x12345678
  vmov s7, r1
  vmov r2, s7
  out r2                         @ [118] 0x12345678
  li r1, 0x11111111
  li r2, 0x22222222
  vmov s8, s9, r1, r2
  outs s8                        @ [119] 0x11111111
  outs s9                        @ [120] 0x22222222
  vmov r3, r4, s8, s9
  out r3                         @ [121]
  out r4                         @ [122]
  li r0, 0xffffffff
  vmsr fpscr, r0
  outfpscr                       @ [123] only the implemented bits stick: 0xf7c0009f
  fpscr_set 0
  vmrs r0, mvfr0
  out r0                         @ [124] 0x10110021
  vmrs r0, mvfr1
  out r0                         @ [125] 0x11000011
  @ ---- loads, stores, block transfers -------------------------------------------------------
  li r5, 0x20000400
  fli s1, 0xcafe0001
  vstr s1, [r5, #4]
  ldr r0, [r5, #4]
  out r0                         @ [126] 0xcafe0001
  vldr s2, [r5, #4]
  outs s2                        @ [127]
  vldr s3, flt_lit
  outs s3                        @ [128] 0x40490fdb
  fli s1, 0x11
  fli s2, 0x22
  fli s3, 0x33
  fli s4, 0x44
  mov r6, r5
  vstmia r6!, {s1-s4}
  out r6                         @ [129] 0x20000410
  ldr r0, [r5, #12]
  out r0                         @ [130] 0x44
  vldmia r5, {s10-s11}
  outs s10                       @ [131] 0x11
  outs s11                       @ [132] 0x22
  mov r6, sp
  fli s16, 0xaaaa0016
  fli s31, 0xaaaa001f
  vpush {s16-s31}
  mov r7, sp
  subs r7, r6, r7
  out r7                         @ [133] 64 bytes pushed
  fli s16, 0
  fli s31, 0
  vpop {s16-s31}
  outs s16                       @ [134] 0xaaaa0016
  outs s31                       @ [135] 0xaaaa001f
  mov r7, sp
  subs r7, r7, r6
  out r7                         @ [136] 0: balanced
  @ ---- CONTROL.FPCA can be cleared by software ----------------------------------------------
  movs r1, #0
  msr control, r1
  isb
  mrs r0, control
  out r0                         @ [137] 0
.global done
done:
  bkpt #0

.balign 4
flt_lit:
  .word 0x40490fdb

@ ---- CPACR access check: FP instructions fault with NOCP until the FPU is enabled ----------
func nocp
  setr12
  li r0, 0xe000ed24
  li r1, (1 << 18)
  str r1, [r0]                   @ enable UsageFault
  li r5, 0x20000300
  movs r6, #0
  str r6, [r5]
.global nocp_site
nocp_site:
  vmov s1, r6                    @ faults (CPACR = 0), the handler enables the FPU, retried
  vadd.f32 s0, s1, s1
  vmov r0, s0
  out r0                         @ [0] 0
  ldr r0, [r5]
  out r0                         @ [1] handler ran once
  ldr r0, [r5, #4]
  out r0                         @ [2] CFSR: NOCP = 0x00080000
  ldr r0, [r5, #8]
  out r0                         @ [3] stacked PC = nocp_site
.global done_nocp
done_nocp:
  bkpt #0

func usage_h
  li r5, 0x20000300
  ldr r1, [r5]
  adds r1, r1, #1
  str r1, [r5]
  li r3, 0xe000ed28
  ldr r1, [r3]
  str r1, [r5, #4]
  li r2, 0xffffffff
  str r2, [r3]                   @ clear CFSR (write one to clear)
  ldr r1, [sp, #24]
  str r1, [r5, #8]
  li r0, 0xe000ed88
  li r1, 0x00f00000
  str r1, [r0]
  dsb
  isb
  bx lr

func dflt
  bkpt #0xff
