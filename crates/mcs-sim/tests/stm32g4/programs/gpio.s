.include "common.inc"
vectors reset=reset

func reset
@ PA5 output: set, reset, then ODR writes; the IDR reads are stored for the test.
func gpio_out
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_AHB2ENR]      @ GPIOAEN
  ldr r1, [r0, #RCC_AHB2ENR]      @ read back (bus delay on silicon)
  li r0, GPIOA
  li r1, 0xABFFF7FF               @ reset MODER with PA5 = output (01)
  str r1, [r0, #GPIO_MODER]
  li r4, 0x20000100
  movs r1, #0x20
  str r1, [r0, #GPIO_BSRR]        @ PA5 high
  ldr r2, [r0, #GPIO_IDR]
  str r2, [r4, #0]
  nop
  nop
  nop
  nop
  li r1, 0x200000
  str r1, [r0, #GPIO_BSRR]        @ PA5 low (BR5)
  ldr r2, [r0, #GPIO_IDR]
  str r2, [r4, #4]
  nop
  nop
  movs r1, #0x20
  str r1, [r0, #GPIO_ODR]         @ ODR write: high
  nop
  movs r1, #0
  str r1, [r0, #GPIO_ODR]         @ low
  ldr r2, [r0, #GPIO_ODR]
  str r2, [r4, #8]
.global out_done
out_done:
  bkpt #0
  b out_done

@ PA0 input with pull-up; IDR is sampled, the test changes the outside world at each BKPT.
func gpio_in
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_AHB2ENR]
  ldr r1, [r0, #RCC_AHB2ENR]
  li r0, GPIOA
  li r1, 0xABFFFFFC               @ PA0 = input
  str r1, [r0, #GPIO_MODER]
  li r1, 0x64000001               @ PA0 pull-up (plus the reset pulls of PA13-15)
  str r1, [r0, #GPIO_PUPDR]
  li r4, 0x20000100
  ldr r2, [r0, #GPIO_IDR]
  str r2, [r4, #0]
.global in_a
in_a:
  bkpt #0
  ldr r2, [r0, #GPIO_IDR]
  str r2, [r4, #4]
.global in_b
in_b:
  bkpt #0
  ldr r2, [r0, #GPIO_IDR]
  str r2, [r4, #8]
.global in_c
in_c:
  bkpt #0
  b in_c

default_handler
