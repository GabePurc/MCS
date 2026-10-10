.include "common.inc"
vectors reset=reset

@ Entry points are selected by the test (it sets PC after reset); each ends in BKPT.
func reset
  setr12
  bkpt #0

@ IT blocks: conditional execution, 16-bit data processing inside IT does not set flags.
func it_basic
  setr12
  movs r0, #5
  movs r3, #0
  movs r4, #0
  cmp r0, #5
  itete eq
  moveq r1, #1
  movne r1, #2
  moveq r2, #3
  movne r2, #4
  out r1
  out r2
  cmp r0, #6
  itt hi
  movhi r3, #9
  movhi r4, #9
  out r3
  out r4
  movs r5, #0
  cmp r5, #0
  it eq
  addeq r5, #7
  mrs r6, apsr
  out r5
  lsrs r6, r6, #28
  out r6
  cmp r0, #5
  ittee ne
  movne r7, #1
  movne r8, #2
  moveq r7, #3
  moveq r8, #4
  out r7
  out r8
  bkpt #0

@ 100 iterations of SUBS/BNE: 1 + 100*1 + 99*3 + 1 + 1(bkpt) = 400 cycles.
func loop_bne
  movs r0, #100
1:
  subs r0, r0, #1
  bne 1b
  bkpt #0

@ CBZ loop: 1 + 10*(1 + 1 + 3) + 3 + 1 = 55 cycles.
func loop_cbz
  movs r0, #10
2:
  cbz r0, 3f
  subs r0, r0, #1
  b 2b
3:
  bkpt #0

@ Call / return and PC loads: BL, BX LR, PUSH {lr}, POP {pc}.
func call_ret
  setr12
  movs r0, #3
  bl double
  out r0
  bl nested
  out r0
  bkpt #0

lfunc double
  adds r0, r0, r0
  bx lr

lfunc nested
  push {r4, lr}
  bl double
  bl double
  pop {r4, pc}

@ Conditional branch coverage: every condition code on both outcomes.
func conds
  setr12
  movs r1, #0
  movs r0, #3
  cmp r0, #3
  bne bad
  beq 1f
  b bad
1:
  adds r1, #1
  cmp r0, #4
  bhs bad
  blo 2f
  b bad
2:
  adds r1, #1
  cmp r0, #2
  bls bad
  bhi 3f
  b bad
3:
  adds r1, #1
  movs r2, #0
  subs r2, r2, #1
  bpl bad
  bmi 4f
  b bad
4:
  adds r1, #1
  movs r0, #0x80
  lsls r0, r0, #24
  subs r2, r0, #1
  bvc bad
  bvs 5f
  b bad
5:
  adds r1, #1
  movs r0, #1
  cmp r0, #2
  bge bad
  blt 6f
  b bad
6:
  adds r1, #1
  cmp r0, #1
  bgt bad
  ble 7f
  b bad
7:
  adds r1, #1
  out r1
  bkpt #0
bad:
  bkpt #0xee

func dflt
  bkpt #0xff
