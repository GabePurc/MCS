.include "common.inc"
vectors reset=reset, irq0=irq0_h

@ A test peripheral at 0x4000_0000: writing offset 0 schedules an event after that many cycles
@ which raises IRQ0; reading offset 4 returns the number of events and clears the IRQ line.
func reset
  li r0, 0xe000e100
  movs r1, #1
  str r1, [r0]
  li r0, 0x40000000
  li r1, 500
.global arm_timer
arm_timer:
  str r1, [r0]
1:
  b 1b

func irq0_h
  li r2, 0x20000200
  li r0, 0x40000000
  ldr r1, [r0, #4]
  str r1, [r2]
.global done
done:
  bkpt #0

func dflt
  bkpt #0xff
