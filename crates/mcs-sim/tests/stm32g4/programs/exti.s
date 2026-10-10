.include "common.inc"
vectors reset=reset, exti0=exti0_h

@ PA0 rising edge -> EXTI0 interrupt; the handler counts at 0x20000200 and clears the pending bit.
func reset
  li r0, RCC
  li r1, 1 | (1 << 0)
  str r1, [r0, #RCC_AHB2ENR]      @ GPIOAEN
  li r1, 1
  str r1, [r0, #RCC_APB2ENR]      @ SYSCFGEN
  ldr r1, [r0, #RCC_APB2ENR]
  li r0, GPIOA
  li r1, 0xABFFFFFC               @ PA0 = input (reset MODER otherwise)
  str r1, [r0, #GPIO_MODER]
  li r0, SYSCFG
  movs r1, #0
  str r1, [r0, #8]                @ EXTICR1: line 0 = PA0
  li r0, EXTI
  movs r1, #1
  str r1, [r0, #8]                @ RTSR1 line 0
  ldr r2, [r0, #0]
  orr r2, r2, #1
  str r2, [r0, #0]                @ IMR1 line 0
  li r0, NVIC_ISER0
  movs r1, #(1 << 6)              @ IRQ 6 = EXTI0
  str r1, [r0]
.global ready
ready:
loop:
  wfi
  b loop

.thumb_func
exti0_h:
  li r2, 0x20000200
  ldr r3, [r2]
  adds r3, r3, #1
  str r3, [r2]
  li r0, EXTI
  movs r1, #1
  str r1, [r0, #0x14]             @ clear PR1 line 0
  bx lr

default_handler
