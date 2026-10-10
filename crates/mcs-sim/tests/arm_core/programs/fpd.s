@ cpu: cortex-m7
.include "common.inc"
vectors reset=reset

@ FPv5-D16 (Cortex-M7): double precision arithmetic, D / S register aliasing, conversions between
@ the formats, VSEL, VMAXNM / VMINNM, VRINT*, VCVTA / N / P / M, VMOV scalar.
@ Expected values are listed in tests/arm_core/main.rs (derived by hand from IEEE 754 and the
@ ARM ARM pseudocode).

.macro fenable
  li r0, 0xe000ed88
  li r1, 0x00f00000
  str r1, [r0]
  dsb
  isb
.endm

@ Loads a 64-bit pattern into a D register.
.macro dli d, hi, lo
  li r0, \lo
  li r1, \hi
  vmov \d, r0, r1
.endm
.macro fli s, v
  li r0, \v
  vmov \s, r0
.endm
@ Output a D register as low word, high word.
.macro outd d
  vmov r0, r1, \d
  out r0
  out r1
.endm
.macro outs s
  vmov r0, \s
  out r0
.endm
.macro outfl
  vmrs r0, fpscr
  and r0, r0, #0x9f
  out r0
.endm
.macro fpscr_set v
  li r0, \v
  vmsr fpscr, r0
.endm
.macro outnzcv
  vmrs APSR_nzcv, fpscr
  mrs r0, apsr
  lsrs r0, r0, #28
  out r0
.endm

func reset
  setr12
  fenable
  @ ---- arithmetic ---------------------------------------------------------------------------
  fpscr_set 0
  dli d1, 0x3ff80000, 0          @ 1.5
  dli d2, 0x40020000, 0          @ 2.25
  vadd.f64 d0, d1, d2
  outd d0                        @ [0,1] 3.75
  vsub.f64 d0, d1, d2
  outd d0                        @ [2,3] -0.75
  vmul.f64 d0, d1, d2
  outd d0                        @ [4,5] 3.375
  outfl                          @ [6] exact
  dli d3, 0x3ff00000, 0
  dli d4, 0x40080000, 0
  vdiv.f64 d0, d3, d4
  outd d0                        @ [7,8] 1/3
  outfl                          @ [9] inexact
  fpscr_set 0
  dli d5, 0x40000000, 0
  vsqrt.f64 d0, d5
  outd d0                        @ [10,11] sqrt(2)
  outfl                          @ [12] inexact
  @ ---- fused versus non-fused ----------------------------------------------------------------
  fpscr_set 0
  dli d1, 0x3ff00000, 0x02000000 @ 1 + 2^-27
  dli d2, 0x3ff00000, 0x02000000
  dli d0, 0xbff00000, 0x04000000 @ -(1 + 2^-26)
  vmla.f64 d0, d1, d2
  outd d0                        @ [13,14] cancels to +0
  dli d0, 0xbff00000, 0x04000000
  vfma.f64 d0, d1, d2
  outd d0                        @ [15,16] 2^-54
  @ ---- rounding modes -----------------------------------------------------------------------
  fpscr_set 0x00400000           @ toward +inf
  vdiv.f64 d0, d3, d4
  outd d0                        @ [17,18] ...556
  fpscr_set 0x00800000           @ toward -inf
  vdiv.f64 d0, d3, d4
  outd d0                        @ [19,20] ...555
  @ ---- D / S aliasing -----------------------------------------------------------------------
  fpscr_set 0
  li r0, 0x11111111
  li r1, 0x22222222
  vmov d1, r0, r1
  vmov r2, s2
  out r2                         @ [21] low word of D1 = S2
  vmov r2, s3
  out r2                         @ [22] high word = S3
  li r3, 0x33333333
  vmov s4, r3
  vmov s5, r0
  outd d2                        @ [23,24] 0x33333333, 0x11111111
  li r1, 0x22222222
  vmov.32 d3[1], r1
  vmov.32 d3[0], r3
  outd d3                        @ [25,26] 0x33333333, 0x22222222
  vmov.32 r4, d3[1]
  out r4                         @ [27] 0x22222222
  @ ---- conversions ----------------------------------------------------------------------------
  fli s1, 0x3fc00000
  vcvt.f64.f32 d0, s1
  outd d0                        @ [28,29] 1.5
  fli s1, 0x7fc01234
  vcvt.f64.f32 d0, s1
  outd d0                        @ [30,31] NaN payload moves up: 0x7ff8024680000000
  dli d6, 0x3fd55555, 0x55555555 @ 0.333...
  fpscr_set 0
  vcvt.f32.f64 s2, d6
  outs s2                        @ [32] 0x3eaaaaab
  outfl                          @ [33] inexact
  fpscr_set 0
  dli d7, 0x40059999, 0x9999999a @ 2.7
  vcvt.s32.f64 s0, d7
  outs s0                        @ [34] 2
  outfl                          @ [35] inexact
  fpscr_set 0
  dli d7, 0x41f2a05f, 0x20000000 @ 5.0e9
  vcvt.u32.f64 s0, d7
  outs s0                        @ [36] saturates: 0xffffffff
  outfl                          @ [37] invalid operation
  fpscr_set 0
  fli s1, 0xfffffffb
  vcvt.f64.s32 d0, s1
  outd d0                        @ [38,39] -5.0
  fli s1, 0xffffffff
  vcvt.f64.u32 d0, s1
  outd d0                        @ [40,41] 4294967295.0
  outfl                          @ [42] exact
  dli d0, 0x3ff80000, 0          @ 1.5
  vcvt.s32.f64 d0, d0, #8
  outs s0                        @ [43] 384
  vcvt.f64.s32 d0, d0, #8
  outd d0                        @ [44,45] 1.5 again
  fli s1, 0x3e00
  vcvtb.f64.f16 d0, s1
  outd d0                        @ [46,47] 1.5
  fli s5, 0
  vcvtt.f16.f64 s5, d0
  outs s5                        @ [48] 0x3e000000
  @ ---- VMAXNM / VMINNM ----------------------------------------------------------------------
  fpscr_set 0
  dli d1, 0x3ff00000, 0          @ 1.0
  dli d2, 0x7ff80000, 0          @ quiet NaN
  vmaxnm.f64 d0, d1, d2
  outd d0                        @ [49,50] the number wins: 1.0
  vminnm.f64 d0, d2, d1
  outd d0                        @ [51,52] 1.0
  outfl                          @ [53] no exception for a quiet NaN
  dli d1, 0x80000000, 0          @ -0.0
  dli d2, 0, 0                   @ +0.0
  vmaxnm.f64 d0, d1, d2
  outd d0                        @ [54,55] +0
  vminnm.f64 d0, d1, d2
  outd d0                        @ [56,57] -0
  fli s1, 0x40000000
  fli s2, 0x40400000
  vminnm.f32 s0, s1, s2
  outs s0                        @ [58] 2.0
  vmaxnm.f32 s0, s1, s2
  outs s0                        @ [59] 3.0
  @ ---- VRINT ----------------------------------------------------------------------------------
  dli d1, 0xbffb3333, 0x33333333 @ -1.7
  vrintz.f64 d0, d1
  outd d0                        @ [60,61] -1.0
  dli d1, 0x3ff33333, 0x33333333 @ 1.2
  vrintp.f64 d0, d1
  outd d0                        @ [62,63] 2.0
  dli d1, 0xbff33333, 0x33333333 @ -1.2
  vrintm.f64 d0, d1
  outd d0                        @ [64,65] -2.0
  dli d1, 0x40040000, 0          @ 2.5
  vrinta.f64 d0, d1
  outd d0                        @ [66,67] 3.0
  vrintn.f64 d0, d1
  outd d0                        @ [68,69] 2.0
  vrintr.f64 d0, d1
  outd d0                        @ [70,71] 2.0 (current mode: nearest even)
  outfl                          @ [72] VRINTR does not signal inexact
  dli d1, 0x3ff80000, 0          @ 1.5
  vrintx.f64 d0, d1
  outd d0                        @ [73,74] 2.0
  outfl                          @ [75] inexact
  fpscr_set 0
  fli s1, 0x402ccccd             @ 2.7f
  vrintz.f32 s0, s1
  outs s0                        @ [76] 2.0f
  @ ---- VCVTA / N / P / M --------------------------------------------------------------------
  dli d1, 0x40040000, 0          @ 2.5
  vcvta.s32.f64 s0, d1
  outs s0                        @ [77] 3
  vcvtn.s32.f64 s0, d1
  outs s0                        @ [78] 2
  dli d1, 0x3ff19999, 0x9999999a @ 1.1
  vcvtp.s32.f64 s0, d1
  outs s0                        @ [79] 2
  dli d1, 0xbff19999, 0x9999999a @ -1.1
  vcvtm.s32.f64 s0, d1
  outs s0                        @ [80] -2
  fli s1, 0x3f000000             @ 0.5f
  vcvta.u32.f32 s0, s1
  outs s0                        @ [81] 1
  fli s1, 0xc0200000             @ -2.5f
  vcvtn.s32.f32 s0, s1
  outs s0                        @ [82] -2
  outfl                          @ [83] inexact
  @ ---- VSEL ---------------------------------------------------------------------------------
  fpscr_set 0
  dli d1, 0x3ff00000, 0          @ 1.0
  dli d2, 0x40000000, 0          @ 2.0
  vcmp.f64 d1, d2
  vmrs APSR_nzcv, fpscr          @ less: N
  vseleq.f64 d0, d1, d2
  outd d0                        @ [84,85] EQ false -> d2 = 2.0
  vselge.f64 d0, d1, d2
  outd d0                        @ [86,87] GE false -> 2.0
  vselgt.f64 d0, d1, d2
  outd d0                        @ [88,89] GT false -> 2.0
  vselvs.f64 d0, d1, d2
  outd d0                        @ [90,91] VS false -> 2.0
  vcmp.f64 d1, d1
  vmrs APSR_nzcv, fpscr          @ equal: Z C
  vseleq.f64 d0, d1, d2
  outd d0                        @ [92,93] EQ -> d1 = 1.0
  vselge.f64 d0, d2, d1
  outd d0                        @ [94,95] GE -> d2 = 2.0
  vselgt.f64 d0, d1, d2
  outd d0                        @ [96,97] GT false -> 2.0
  dli d3, 0x7ff80000, 0
  vcmp.f64 d1, d3
  outnzcv                        @ [98] unordered: 3
  vmrs APSR_nzcv, fpscr
  fli s1, 0x3f800000
  fli s2, 0x40000000
  vselvs.f32 s0, s1, s2
  outs s0                        @ [99] VS -> s1 = 1.0f
  @ ---- double precision exceptions and flush to zero ----------------------------------------
  fpscr_set 0
  dli d1, 0x7ff00000, 0
  vsub.f64 d0, d1, d1
  outd d0                        @ [100,101] default NaN
  outfl                          @ [102] invalid operation
  fpscr_set 0
  dli d1, 0x3ff00000, 0
  dli d2, 0, 0
  vdiv.f64 d0, d1, d2
  outd d0                        @ [103,104] +inf
  outfl                          @ [105] divide by zero
  fpscr_set 0
  dli d1, 0x7ff80000, 0x1234
  dli d2, 0x3ff00000, 0
  vadd.f64 d0, d1, d2
  outd d0                        @ [106,107] payload kept
  dli d1, 0, 1                   @ smallest denormal
  vadd.f64 d0, d1, d1
  outd d0                        @ [108,109] 2
  outfl                          @ [110] exact
  fpscr_set 0x01000000
  vadd.f64 d0, d1, d1
  outd d0                        @ [111,112] flushed
  outfl                          @ [113] IDC
  fpscr_set 0
  dli d1, 0x7fefffff, 0xffffffff
  vadd.f64 d0, d1, d1
  outd d0                        @ [114,115] +inf
  outfl                          @ [116] overflow + inexact
  @ ---- memory --------------------------------------------------------------------------------
  li r5, 0x20000400
  dli d1, 0x01234567, 0x89abcdef
  vstr d1, [r5, #8]
  ldr r0, [r5, #8]
  out r0                         @ [117] low word first
  ldr r0, [r5, #12]
  out r0                         @ [118]
  vldr d2, [r5, #8]
  outd d2                        @ [119,120]
  dli d8, 0x00000008, 0x00000008
  dli d15, 0x0000000f, 0x0000000f
  mov r6, sp
  vpush {d8-d15}
  mov r7, sp
  subs r7, r6, r7
  out r7                         @ [121] 64 bytes
  dli d8, 0, 0
  dli d15, 0, 0
  vpop {d8-d15}
  outd d8                        @ [122,123]
  outd d15                       @ [124,125]
  vmrs r0, mvfr0
  out r0                         @ [126] 0x10110221
  vmrs r0, mvfr1
  out r0                         @ [127] 0x12000011
  vmrs r0, mvfr2
  out r0                         @ [128] 0x00000040
.global done
done:
  bkpt #0

func dflt
  bkpt #0xff
