.include "common.inc"
vectors reset=reset, exti0=exti0_h

@ Does nothing; the tests drive the peripherals by poking registers from Rust.
func reset
.global idle
idle:
  b idle

.thumb_func
exti0_h:
  bx lr

default_handler
