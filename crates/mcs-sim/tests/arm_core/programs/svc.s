.include "common.inc"
vectors reset=reset, svc=svc_h, pendsv=pendsv_h

@ SVC #5 with r0 = 10: the handler decodes the immediate from the stacked PC, adds it to the
@ stacked r0 and pends PendSV (lowest priority). PendSV runs after SVC returns (tail-chain) and
@ before the thread continues.
func reset
  setr12
  li r0, 0xe000ed20
  li r1, 0x00f00000
  str r1, [r0]
  movs r0, #10
.global svc_site
svc_site:
  svc #5
  out r0
  li r2, 0x20000300
  ldr r1, [r2]
  out r1
  ldr r1, [r2, #4]
  out r1
.global done
done:
  bkpt #0

func svc_h
  ldr r0, [sp, #24]
  ldrb r1, [r0, #-2]
  ldr r2, [sp]
  adds r2, r2, r1
  str r2, [sp]
  li r3, 0xe000ed04
  li r1, 0x10000000
  str r1, [r3]
  li r2, 0x20000300
  movs r1, #1
  str r1, [r2]
  bx lr

@ Records that SVC ran first (0x20000300 == 1) by storing 2 in 0x20000304.
func pendsv_h
  li r2, 0x20000300
  ldr r1, [r2]
  adds r1, #1
  str r1, [r2, #4]
  bx lr

func dflt
  bkpt #0xff
