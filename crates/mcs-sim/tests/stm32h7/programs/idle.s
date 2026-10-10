.include "common.inc"
vectors reset=reset

@ Does nothing; the tests drive the peripherals by poking registers from Rust.
func reset
.global idle
idle:
  b idle

default_handler
