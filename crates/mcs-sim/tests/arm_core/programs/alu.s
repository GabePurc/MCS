.include "common.inc"
vectors reset=reset

func reset
  setr12
  @ 1. signed overflow
  mvn r1, #0x80000000
  adds r2, r1, #1
  mrs r3, apsr
  out r2
  out r3
  @ 2. borrow
  movs r5, #0
  subs r6, r5, #1
  mrs r7, apsr
  out r6
  out r7
  @ 3. carry and zero
  mvn r8, #0
  adds r9, r8, #1
  mrs r10, apsr
  out r9
  out r10
  @ 4. ADC / SBC with C set and clear
  movs r0, #5
  cmp r0, r0
  adc r11, r0, #10
  out r11
  cmp r0, r0
  sbc r1, r0, #2
  out r1
  cmp r0, #9
  sbc r1, r0, #2
  out r1
  @ 5. shifts
  movs r0, #0x81
  lsls r1, r0, #25
  mrs r2, apsr
  out r1
  out r2
  movs r0, #1
  lsls r0, r0, #31
  asrs r3, r0, #31
  lsrs r4, r0, #31
  li r5, 0x12345678
  movs r6, #8
  rors r5, r6
  out r3
  out r4
  out r5
  movs r0, #1
  cmp r0, #0
  rrx r1, r0
  out r1
  @ 6. logic with modified immediates
  movs r0, #0xf0
  and r1, r0, #0x3c
  orr r2, r0, #0x0f
  eor r3, r0, #0xff
  bic r4, r0, #0x30
  orn r5, r0, #0xf
  out r1
  out r2
  out r3
  out r4
  out r5
  @ 7. multiply / divide
  movs r0, #7
  movs r1, #6
  mul r2, r0, r1
  mla r3, r0, r1, r0
  mls r4, r0, r1, r3
  out r2
  out r3
  out r4
  mvn r0, #0
  movs r1, #3
  umull r2, r3, r0, r1
  smull r4, r5, r0, r1
  out r2
  out r3
  out r4
  out r5
  umlal r2, r3, r0, r1
  smlal r4, r5, r0, r1
  out r2
  out r3
  out r4
  out r5
  movw r0, #1000
  movs r1, #7
  udiv r2, r0, r1
  rsbs r0, r0, #0
  sdiv r3, r0, r1
  movs r1, #0
  udiv r4, r0, r1
  sdiv r5, r0, r1
  out r2
  out r3
  out r4
  out r5
  @ 8. bit fields and byte reversal
  li r0, 0x1234abcd
  ubfx r1, r0, #4, #8
  sbfx r2, r0, #4, #8
  sbfx r3, r0, #8, #4
  movs r4, #0xff
  bfi r4, r0, #8, #4
  out r1
  out r2
  out r3
  out r4
  bfc r4, #0, #4
  clz r5, r0
  rbit r6, r0
  rev r7, r0
  rev16 r8, r0
  revsh r9, r0
  out r4
  out r5
  out r6
  out r7
  out r8
  out r9
  @ 9. extend
  uxtb r1, r0
  sxtb r2, r0
  uxth r3, r0
  sxth r4, r0
  uxtb r5, r0, ror #8
  out r1
  out r2
  out r3
  out r4
  out r5
  @ 10. saturate
  movw r0, #1000
  ssat r1, #8, r0
  mrs r2, apsr
  and r2, r2, #0x08000000
  usat r3, #8, r0
  movs r0, #0
  subs r0, r0, #5
  usat r4, #8, r0
  ssat r5, #8, r0
  mvn r0, #0x80000000
  qadd r6, r0, r0
  out r1
  out r2
  out r3
  out r4
  out r5
  out r6
.global done
done:
  bkpt #0

func dflt
  bkpt #0xff
