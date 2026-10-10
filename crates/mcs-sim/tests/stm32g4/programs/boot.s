.include "common.inc"
vectors reset=reset

@ Writes a marker to SRAM and stops.
func reset
  li r0, 0x20000100
  li r1, 0xcafebabe
  str r1, [r0]
.global done
done:
  bkpt #0
  b done

default_handler
