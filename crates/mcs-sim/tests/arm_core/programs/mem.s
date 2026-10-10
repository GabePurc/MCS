.include "common.inc"
vectors reset=reset

func reset
  setr12
  li r11, 0x20000400
  @ STM / LDMDB with write-back
  movs r0, #1
  movs r1, #2
  movs r2, #3
  movs r3, #4
  stmia r11!, {r0-r3}
  ldmdb r11!, {r4-r7}
  out r4
  out r5
  out r6
  out r7
  out r11
  @ PUSH / POP
  movs r0, #0xaa
  movs r1, #0xbb
  push {r0, r1, lr}
  mov r2, sp
  out r2
  movs r0, #0
  movs r1, #0
  pop {r0, r1, r3}
  mov r2, sp
  out r0
  out r1
  out r2
  @ LDRD / STRD and sub-word loads
  li r0, 0x11112222
  li r1, 0x8070ff80
  strd r0, r1, [r11, #8]
  ldrd r2, r3, [r11, #8]
  out r2
  out r3
  ldrh r5, [r11, #10]
  ldrb r6, [r11, #9]
  ldrsb r7, [r11, #12]
  ldrsh r8, [r11, #12]
  ldr r9, [r11, #9]
  out r5
  out r6
  out r7
  out r8
  out r9
  @ pre / post indexing and negative offsets
  li r0, 0x20000600
  li r1, 0xdeadbeef
  str r1, [r0, #4]!
  out r0
  ldr r2, [r0], #8
  out r2
  out r0
  ldr r3, [r0, #-8]
  out r3
  movs r6, #2
  ldr r4, [r0, r6, lsl #2]
  out r4
  @ literal pool access
  ldr r5, lit
  adr r6, lit
  ldr r7, [r6]
  out r5
  out r7
  @ TBB / TBH
  movs r0, #2
  tbb [pc, r0]
tbl1:
  .byte (c0 - tbl1) / 2, (c1 - tbl1) / 2, (c2 - tbl1) / 2
  .balign 2
c0:
  movs r5, #10
  b join1
c1:
  movs r5, #11
  b join1
c2:
  movs r5, #12
join1:
  out r5
  movs r0, #1
  tbh [pc, r0, lsl #1]
tbl2:
  .hword (d0 - tbl2) / 2, (d1 - tbl2) / 2, (d2 - tbl2) / 2
d0:
  movs r5, #20
  b join2
d1:
  movs r5, #21
  b join2
d2:
  movs r5, #22
join2:
  out r5
  @ LDREX / STREX
  li r0, 0x20000500
  movs r1, #7
  str r1, [r0]
  ldrex r2, [r0]
  adds r2, #1
  strex r3, r2, [r0]
  ldr r4, [r0]
  strex r5, r2, [r0]
  ldrex r6, [r0]
  clrex
  strex r7, r6, [r0]
  out r2
  out r3
  out r4
  out r5
  out r7
.global done
done:
  bkpt #0

.balign 4
lit:
  .word 0xcafef00d

func dflt
  bkpt #0xff
