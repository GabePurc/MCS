.include "common.inc"
vectors reset=reset, hard=hard_h, usage=usage_h, bus=bus_h, irq0=irq0_bad

@ Fault handlers dump HFSR / CFSR / BFAR / stacked PC to 0x20000300.. and the handler id to
@ 0x20000310, then stop with a BKPT. Entry points are selected by the test.
.macro dump id
  li r2, 0x20000300
  li r3, 0xe000ed2c
  ldr r1, [r3]
  str r1, [r2]
  ldr r1, [r3, #-4]
  str r1, [r2, #4]
  ldr r1, [r3, #12]
  str r1, [r2, #8]
  ldr r1, [sp, #24]
  str r1, [r2, #12]
  movs r1, #\id
  str r1, [r2, #16]
.endm

func reset
  bkpt #0

.macro enable bit
  li r0, 0xe000ed24
  li r1, (1 << \bit)
  str r1, [r0]
.endm

func udf_hard
  setr12
.global udf_hard_site
udf_hard_site:
  udf #0x12
  bkpt #0xee

func udf_usage
  setr12
  enable 18
.global udf_usage_site
udf_usage_site:
  udf #1
  bkpt #0xee

func bus_hard
  setr12
  li r0, 0x60000000
.global bus_hard_site
bus_hard_site:
  ldr r1, [r0]
  bkpt #0xee

func bus_bus
  setr12
  enable 17
  li r0, 0x60000010
.global bus_bus_site
bus_bus_site:
  str r1, [r0]
  bkpt #0xee

func div0
  setr12
  enable 18
  li r0, 0xe000ed14
  ldr r1, [r0]
  orr r1, r1, #0x10
  str r1, [r0]
  movs r2, #0
  movs r1, #5
.global div0_site
div0_site:
  udiv r3, r1, r2
  bkpt #0xee

@ A fault while HardFault is active locks the core up.
func lockup
  setr12
  li r2, 0x20000330
  movs r1, #1
  str r1, [r2]
  udf #0
  bkpt #0xee

@ Exception return with an invalid EXC_RETURN value raises a UsageFault (INVPC); IRQ0 runs at the
@ same priority as UsageFault (0), so the fault escalates to HardFault.
func bad_return
  setr12
  enable 18
  li r0, 0xe000e100
  movs r1, #1
  str r1, [r0]
  li r0, 0xe000e200
  str r1, [r0]
  bkpt #0xee

func irq0_bad
  mvn r0, #10
.global bad_return_site
bad_return_site:
  bx r0

func hard_h
  li r2, 0x20000330
  ldr r1, [r2]
  cmp r1, #0
  bne 1f
  dump 3
  bkpt #0
1:
  udf #0

func usage_h
  dump 6
  bkpt #0

func bus_h
  dump 5
  bkpt #0

func dflt
  bkpt #0xff
