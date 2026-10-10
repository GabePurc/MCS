.include "common.inc"
vectors reset=reset, systick=systick_h

@ SysTick with RVR = 99 on the core clock: an interrupt every 100 cycles. The handler counts
@ ticks at 0x20000200 and stops (BKPT) on the third.
func reset
  setr12
  li r0, 0xe000e010
  movs r1, #99
  str r1, [r0, #4]
  movs r1, #0
  str r1, [r0, #8]
  movs r1, #7
.global enable
enable:
  str r1, [r0]
.global spin
spin:
  b spin

.global systick_h
.thumb_func
systick_h:
  li r2, 0x20000200
  ldr r3, [r2]
  adds r3, r3, #1
  str r3, [r2]
  cmp r3, #3
  bne 1f
  bkpt #0
1:
  bx lr

@ WFI until the SysTick fires 1000 cycles after enabling it; the handler sets 0x20000204.
func wfi_main
  setr12
  li r0, 0xe000e010
  li r1, 999
  str r1, [r0, #4]
  movs r1, #0
  str r1, [r0, #8]
  movs r1, #7
.global wfi_enable
wfi_enable:
  str r1, [r0]
  wfi
.global wfi_after
wfi_after:
  bkpt #0

@ Reads CVR / CSR directly: countflag clears on read.
func read_back
  setr12
  li r0, 0xe000e010
  movs r1, #49
  str r1, [r0, #4]
  movs r1, #0
  str r1, [r0, #8]
  movs r1, #5
  str r1, [r0]
  nop
  nop
  ldr r2, [r0, #8]
  out r2
  .rept 60
  nop
  .endr
  ldr r3, [r0]
  ldr r4, [r0]
  ldr r5, [r0, #8]
  out r3
  out r4
  out r5
  bkpt #0

func dflt
  bkpt #0xff
