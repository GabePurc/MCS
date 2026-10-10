.include "common.inc"
vectors reset=reset

@ Touches every RAM region of the STM32H743 and runs a function copied into the ITCM.
@ Results (DTCM 0x20000100..): sum read back from AXI SRAM, SRAM1, SRAM4, backup SRAM, the SRAM1 alias at
@ 0x10000000, then the value returned by the ITCM function.
func reset
  li r0, 0x20000100
  li r1, 0x24000000           @ AXI SRAM
  li r2, 0x11111111
  str r2, [r1, #0x100]
  ldr r3, [r1, #0x100]
  str r3, [r0, #0]
  li r1, 0x30000000           @ SRAM1
  li r2, 0x22222222
  str r2, [r1, #0x20]
  ldr r3, [r1, #0x20]
  str r3, [r0, #4]
  li r1, 0x38000000           @ SRAM4
  li r2, 0x33333333
  str r2, [r1]
  ldr r3, [r1]
  str r3, [r0, #8]
  li r1, 0x38800000           @ backup SRAM
  li r2, 0x44444444
  str r2, [r1]
  ldr r3, [r1]
  str r3, [r0, #12]
  li r1, 0x10000020           @ SRAM1 through the 0x1000_0000 alias
  ldr r3, [r1]
  str r3, [r0, #16]
  @ Copy "movs r0, #42; bx lr" into the ITCM and call it.
  li r1, 0x00000000
  li r2, 0x4770202a           @ halfwords: movs r0,#42 (0x202a), bx lr (0x4770)
  str r2, [r1]
  movs r4, #1
  orrs r1, r1, r4             @ Thumb bit
  blx r1
  li r1, 0x20000114
  str r0, [r1]
.global done
done:
  bkpt #0
  b done

default_handler
