.include "common.inc"
vectors reset=reset

@ Tight loop for the throughput test: SUBS (1) + BNE taken (3) = 4 cycles per iteration,
@ 12.5 M iterations = 50 M cycles.
func reset
  li r0, 12500000
1:
  subs r0, r0, #1
  bne 1b
.global done
done:
  bkpt #0

@ Mixed workload: load / store / ALU / multiply / call in a 24-cycle iteration body.
func mixed
  li r0, 2000000
  li r4, 0x20000800
  movs r1, #0
1:
  ldr r2, [r4]
  adds r2, r2, #3
  muls r2, r1, r2
  eors r2, r2, r0
  str r2, [r4]
  ldr r3, [r4, #4]
  adds r3, r3, r2
  str r3, [r4, #4]
  adds r1, #1
  bl leaf
  subs r0, r0, #1
  bne 1b
  bkpt #0

lfunc leaf
  adds r1, r1, #1
  bx lr

func dflt
  bkpt #0xff
