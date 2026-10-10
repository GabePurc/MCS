.include "common.inc"
vectors reset=reset

@ ARMv7E-M DSP extension: parallel arithmetic with the GE flags, SEL, USAD8, saturation, packing,
@ extension, the signed multiplies (with the Q flag) and APSR GE / Q access through MRS / MSR.
@ Expected values are listed in tests/arm_core/main.rs (derived by hand from the ARM ARM).

@ Outputs the GE field (APSR[19:16]).
.macro showge
  mrs r11, apsr
  ubfx r11, r11, #16, #4
  out r11
.endm

@ Outputs the Q flag (APSR[27]).
.macro showq
  mrs r11, apsr
  ubfx r11, r11, #27, #1
  out r11
.endm

.macro clearq
  movs r11, #0
  msr apsr_nzcvq, r11
.endm

func reset
  setr12
  @ ---- parallel add / subtract and GE -------------------------------------------------------
  li r1, 0xff010203
  li r2, 0x01ff0304
  uadd8 r0, r1, r2
  out r0
  showge
  li r1, 0x00010005
  li r2, 0x00020003
  ssub16 r0, r1, r2
  out r0
  showge
  li r1, 0x00030004
  li r2, 0x00010002
  sasx r0, r1, r2
  out r0
  showge
  li r1, 0x7fff0001
  li r2, 0x00010001
  sadd16 r0, r1, r2
  out r0
  showge
  li r1, 0x7fff8000
  li r2, 0x00010001
  qadd16 r0, r1, r2
  out r0
  showge                       @ unchanged by the saturating form
  li r1, 0x10203040
  li r2, 0x20102050
  uqsub8 r0, r1, r2
  out r0
  li r1, 0x7fff0003
  li r2, 0x7fff0004
  shadd16 r0, r1, r2
  out r0
  li r1, 0x01020304
  li r2, 0x04030201
  uhsub8 r0, r1, r2
  out r0
  li r1, 0x00020005
  li r2, 0x00030001
  usax r0, r1, r2          @ lo = 5 + 3, hi = 2 - 1
  out r0
  showge
  @ ---- SEL (GE = 0b1100 from the UADD8 above is gone; build it again) --------------------------
  li r1, 0xff010203
  li r2, 0x01ff0304
  uadd8 r0, r1, r2
  li r1, 0x11223344
  li r2, 0xaabbccdd
  sel r0, r1, r2
  out r0
  @ ---- USAD8 / USADA8 -----------------------------------------------------------------------
  li r1, 0x01020304
  li r2, 0x04030201
  usad8 r0, r1, r2
  out r0
  movs r3, #0x80
  lsls r3, r3, #1
  usada8 r0, r1, r2, r3
  out r0
  @ ---- SSAT16 / USAT16 (Q flag) -------------------------------------------------------------
  clearq
  li r1, 0x012cfed4
  ssat16 r0, #8, r1
  out r0
  showq
  clearq
  usat16 r0, #8, r1
  out r0
  showq
  clearq
  li r1, 0x00050003
  ssat16 r0, #8, r1        @ in range: Q stays clear
  out r0
  showq
  @ ---- pack and extend ----------------------------------------------------------------------
  li r1, 0xaaaa1111
  li r2, 0x2222bbbb
  pkhbt r0, r1, r2, lsl #16
  out r0
  pkhtb r0, r1, r2, asr #16
  out r0
  pkhtb r0, r1, r2, asr #32
  out r0
  li r1, 0x00800080
  sxtb16 r0, r1
  out r0
  li r1, 0x12345678
  uxtb16 r0, r1
  out r0
  uxtb16 r0, r1, ror #8
  out r0
  li r1, 0x00010001
  li r2, 0x00800080
  sxtab16 r0, r1, r2
  out r0
  li r2, 0x00ff00ff
  uxtab16 r0, r1, r2
  out r0
  @ ---- halfword multiplies ------------------------------------------------------------------
  li r1, 0xfffe0003
  li r2, 0x00050007
  smulbb r0, r1, r2
  out r0
  smulbt r0, r1, r2
  out r0
  smultb r0, r1, r2
  out r0
  smultt r0, r1, r2
  out r0
  movs r3, #100
  smlabb r0, r1, r2, r3
  out r0
  clearq
  li r3, 0x7fffffff
  smlabb r0, r1, r2, r3    @ 21 + 0x7fffffff overflows: wraps and sets Q
  out r0
  showq
  li r1, 0x00010000
  li r2, 0x00038000
  smulwb r0, r1, r2
  out r0
  smulwt r0, r1, r2
  out r0
  movs r3, #1
  smlawt r0, r1, r2, r3
  out r0
  li r1, 0xfffe0003
  li r2, 0x00050007
  li r3, 0x00000001
  li r4, 0x00000001
  smlalbb r3, r4, r1, r2   @ RdHi:RdLo = 0x1_00000001 + 21
  out r3
  out r4
  li r3, 0x00000001
  li r4, 0x00000000
  smlaltt r3, r4, r1, r2   @ 1 - 10
  out r3
  out r4
  @ ---- dual multiplies ----------------------------------------------------------------------
  li r1, 0x00030002
  li r2, 0x00050004
  smuad r0, r1, r2
  out r0
  smuadx r0, r1, r2
  out r0
  smusd r0, r1, r2
  out r0
  smusdx r0, r1, r2
  out r0
  movs r3, #100
  smlad r0, r1, r2, r3
  out r0
  movs r3, #10
  smlsdx r0, r1, r2, r3
  out r0
  clearq
  li r1, 0x80008000
  smuad r0, r1, r1
  out r0
  showq
  li r1, 0x00030002
  li r2, 0x00050004
  movs r3, #1
  movs r4, #0
  smlald r3, r4, r1, r2
  out r3
  out r4
  movs r3, #1
  movs r4, #0
  smlsld r3, r4, r1, r2
  out r3
  out r4
  @ ---- most significant word multiplies -----------------------------------------------------
  li r1, 0x40000000
  smmul r0, r1, r1
  out r0
  li r1, 0x7fffffff
  movs r2, #2
  smmul r0, r1, r2
  out r0
  smmulr r0, r1, r2
  out r0
  movs r3, #5
  smmla r0, r1, r2, r3
  out r0
  smmls r0, r1, r2, r3
  out r0
  smmlsr r0, r1, r2, r3
  out r0
  @ ---- UMAAL --------------------------------------------------------------------------------
  li r0, 0xffffffff
  li r1, 0xffffffff
  li r2, 0xffffffff
  umaal r0, r1, r2, r2
  out r0
  out r1
  @ ---- GE / Q through MRS / MSR -------------------------------------------------------------
  clearq
  li r1, 0x00050000
  msr apsr_g, r1
  mrs r0, apsr
  out r0                   @ GE = 0b0101
  li r2, 0xf8000000
  msr apsr_nzcvq, r2
  mrs r0, apsr
  out r0                   @ NZCVQ set, GE kept
  movs r2, #0
  msr apsr_nzcvq, r2
  li r1, 0x000a0000
  msr apsr_nzcvqg, r1
  mrs r0, apsr
  out r0
.global done
done:
  bkpt #0

func dflt
  bkpt #0xff
