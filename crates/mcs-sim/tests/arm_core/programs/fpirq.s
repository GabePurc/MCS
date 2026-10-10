.include "common.inc"
vectors reset=ext_frame, irq0=irq0_h, irq1=irq1_h

@ Exception entry / return with floating-point context (extended 26-word frame, EXC_RETURN bit 4,
@ CONTROL.FPCA, FPCCR). Results are stored at 0x20000300.. and the S registers / FPSCR at the end
@ at 0x20000500 (see tests/arm_core/main.rs).

.macro fenable
  li r0, 0xe000ed88
  li r1, 0x00f00000
  str r1, [r0]
  dsb
  isb
.endm

@ Fills S0-S31 with 0x3f800000 + n.
.macro fillall
  .irp n,0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
  li r0, 0x3f800000 + \n
  vmov s\n, r0
  .endr
.endm

@ IRQ0 priority 0x40, IRQ1 priority 0x20 (higher), both enabled.
.macro irq_setup
  li r0, 0xe000e400
  movs r1, #0x40
  strb r1, [r0]
  movs r1, #0x20
  strb r1, [r0, #1]
  li r0, 0xe000e100
  movs r1, #3
  str r1, [r0]
.endm

@ Pends IRQ \n.
.macro pend n
  li r0, 0xe000e200
  movs r1, #(1 << \n)
  str r1, [r0]
.endm

@ Stores S0-S31, FPSCR, CONTROL and SP at 0x20000500.
.macro dump
  li r5, 0x20000500
  vstmia r5!, {s0-s31}
  vmrs r4, fpscr
  str r4, [r5]
  mrs r4, control
  str r4, [r5, #4]
  mov r4, sp
  str r4, [r5, #8]
.endm

@ Thread with an active FP context takes IRQ0: extended frame, handler clobbers S0-S15 / FPSCR.
func ext_frame
  fenable
  irq_setup
  fillall
  li r0, 0x60400011              @ FPSCR: Z C flags, round toward +inf, IOC + IXC
  vmsr fpscr, r0
  pend 0
  nop
  nop
  dump
.global done
done:
  bkpt #0

@ Without FP state in use (FPCA = 0) the frame is the basic 8-word frame.
func std_frame
  fenable
  irq_setup
  li r6, 0x20000300
  movs r7, #0
  str r7, [r6, #12]
  li r5, 0x20000600
  mov r7, sp
  str r7, [r5]
  pend 0
  nop
  nop
  mov r7, sp
  str r7, [r5, #4]
  mrs r7, control
  str r7, [r5, #8]
.global done_std
done_std:
  bkpt #0

@ IRQ0 handler of the first two entry points. Records CONTROL / FPSCR at entry, then uses FP.
func irq0_h
  mrs r0, control
  li r2, 0x20000300
  str r0, [r2]                   @ [0] CONTROL.FPCA at handler entry (0)
  str lr, [r2, #4]               @ [1] EXC_RETURN
  mov r3, sp
  str r3, [r2, #8]               @ [2] SP in the handler
  ldr r0, [r2, #12]
  cbnz r0, nested_part
  li r0, 0x40000000
  vmov s0, r0
  vmrs r1, fpscr
  str r1, [r2, #16]              @ [4] FPSCR at entry = FPDSCR defaults
  li r0, 0x80c00000              @ N flag, round toward zero
  vmsr fpscr, r0
  vmov s5, r0
  mrs r0, control
  str r0, [r2, #20]              @ [5] FPCA set by the FP instructions
  bx lr
nested_part:
  @ Nested test: IRQ0 handler uses FP, then IRQ1 (higher priority) preempts it.
  li r0, 0x40400000
  vmov s0, r0
  li r0, 0x00400000              @ round toward +inf
  vmsr fpscr, r0
  pend 1
  nop
  nop
  vmov r0, s0
  str r0, [r2, #24]              @ [6] S0 survived the nested exception
  vmrs r0, fpscr
  str r0, [r2, #28]              @ [7] FPSCR survived
  mrs r0, control
  str r0, [r2, #32]              @ [8] FPCA still set
  bx lr

@ IRQ1 handler: clobbers S0 and FPSCR; records its own frame facts.
func irq1_h
  mrs r0, control
  li r2, 0x20000340
  str r0, [r2]                   @ FPCA clear on entry
  str lr, [r2, #4]               @ EXC_RETURN: extended frame from a handler = 0xffffffe1
  li r0, 0x41000000
  vmov s0, r0
  li r0, 0x00c00000
  vmsr fpscr, r0
  bx lr

@ Nested exceptions: the thread has FP state, IRQ0 stacks it, IRQ0 uses FP and IRQ1 stacks that.
func nested
  fenable
  irq_setup
  fillall
  li r0, 0x20400001              @ FPSCR: C flag, round toward +inf, IOC
  vmsr fpscr, r0
  li r6, 0x20000300
  movs r7, #1
  str r7, [r6, #12]              @ select the nested part of the IRQ0 handler
  pend 0
  nop
  nop
  dump
.global done_nested
done_nested:
  bkpt #0

func dflt
  bkpt #0xff
