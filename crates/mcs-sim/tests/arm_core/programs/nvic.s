.include "common.inc"
vectors reset=reset, irq0=irq0_h, irq1=irq1_h, irq2=irq2_h, irq3=irq3_h

@ Appends \val to the log at 0x20000304 (count at 0x20000300). Clobbers r1-r3.
.macro log val
  li r2, 0x20000300
  ldr r3, [r2]
  add r2, r2, r3, lsl #2
  movs r1, #\val
  str r1, [r2, #4]
  adds r3, r3, #1
  li r2, 0x20000300
  str r3, [r2]
.endm

@ ISPR write: pends IRQ number \n (0-31).
.macro pend n
  li r0, 0xe000e200
  movs r1, #(1 << \n)
  str r1, [r0]
.endm

@ Sets priorities IRQ0 = 0x40, IRQ1 = 0x20, IRQ2 = 0x40, IRQ3 = 0x40 and enables IRQ0-3.
.macro setup
  li r0, 0xe000e400
  movs r1, #0x40
  strb r1, [r0, #0]
  movs r1, #0x20
  strb r1, [r0, #1]
  movs r1, #0x40
  strb r1, [r0, #2]
  strb r1, [r0, #3]
  li r0, 0xe000e100
  movs r1, #0xf
  str r1, [r0]
.endm

@ IRQ1 (higher priority) preempts IRQ0 from inside its handler; IRQ2 (same priority as IRQ0)
@ does not, and runs after IRQ0 returns (tail-chained).
func reset
  setr12
  setup
  pend 0
  nop
  nop
.global done
done:
  bkpt #0

@ Same program with AIRCR.PRIGROUP = 6: IRQ0/IRQ1/IRQ2 share one group, so nothing preempts and
@ the sub-priority orders the pending handlers.
func grouped
  setr12
  setup
  li r0, 0xe000ed0c
  li r1, 0x05fa0600
  str r1, [r0]
  pend 0
  nop
  nop
  bkpt #0

@ PRIMASK: a pended IRQ is only taken after CPSIE.
func masked
  setr12
  setup
  cpsid i
  pend 3
  nop
  nop
  li r2, 0x20000320
  movs r1, #9
  str r1, [r2]
  cpsie i
  nop
  bkpt #0

@ BASEPRI = 0x40 masks priorities >= 0x40 (IRQ3) but not IRQ1 (0x20).
func basepri
  setr12
  setup
  movs r1, #0x40
  msr basepri, r1
  pend 3
  pend 1
  nop
  nop
  bkpt #1
  movs r1, #0
  msr basepri, r1
  nop
  bkpt #2

func irq0_h
  log 1
  pend 1
  log 3
  pend 2
  log 4
  bx lr

func irq1_h
  log 2
  bx lr

func irq2_h
  log 5
  bx lr

@ Copies the marker at 0x20000320 to 0x20000324 and counts invocations at 0x20000328.
func irq3_h
  li r2, 0x20000320
  ldr r1, [r2]
  str r1, [r2, #4]
  ldr r1, [r2, #8]
  adds r1, #1
  str r1, [r2, #8]
  bx lr

func dflt
  bkpt #0xff
