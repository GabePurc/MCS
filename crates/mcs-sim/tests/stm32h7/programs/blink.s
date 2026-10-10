.include "common.inc"
vectors reset=reset

@ PB0 (Nucleo LD1) toggles forever with a delay loop between the edges (the checked-in ELF is built
@ from this).
func reset
  li r0, RCC
  movs r1, #2                              @ GPIOBEN
  str r1, [r0, #RCC_AHB4ENR]
  ldr r1, [r0, #RCC_AHB4ENR]
  li r0, GPIOB
  li r1, 0xFFFFFEBD                        @ PB0 output
  str r1, [r0, #GPIO_MODER]
.global blink_loop
blink_loop:
1:
  movs r1, #1
  str r1, [r0, #GPIO_BSRR]
  bl delay
  li r1, 0x10000
  str r1, [r0, #GPIO_BSRR]
  bl delay
  b 1b

.thumb_func
delay:
  movs r2, #100
1:
  subs r2, r2, #1
  bne 1b
  bx lr

default_handler
