.include "common.inc"
vectors reset=reset, exti15=exti15_h

@ Nucleo-H743ZI: user button B1 on PC13 (high while pressed), LD1 = PB0, LD2 = PE1.
@ PC13 rising edge -> EXTI line 13 -> EXTI15_10 interrupt (IRQ 40). The handler counts at
@ 0x20000200, clears PR1 and toggles LD1 / LD2.
func reset
  li r0, RCC
  li r1, (1 << 1) | (1 << 2) | (1 << 4)   @ GPIOBEN | GPIOCEN | GPIOEEN
  str r1, [r0, #RCC_AHB4ENR]
  movs r1, #2                              @ SYSCFGEN
  str r1, [r0, #RCC_APB4ENR]
  ldr r1, [r0, #RCC_APB4ENR]
  li r0, GPIOB
  li r1, 0xFFFFFEBD                        @ PB0 output (PB3/PB4 keep their reset AF)
  str r1, [r0, #GPIO_MODER]
  li r0, GPIOE
  li r1, 0xFFFFFFF7                        @ PE1 output
  str r1, [r0, #GPIO_MODER]
  li r0, GPIOC
  li r1, 0xF3FFFFFF                        @ PC13 input
  str r1, [r0, #GPIO_MODER]
  li r0, SYSCFG
  movs r1, #0x20
  str r1, [r0, #(SYSCFG_EXTICR1 + 12)]     @ EXTICR4: line 13 = PC
  li r0, EXTI
  li r1, 1 << 13
  str r1, [r0, #EXTI_RTSR1]
  ldr r2, [r0, #EXTI_IMR1]                 @ CPU interrupt mask (IMR1 at +0x80), reset value keeps lines 22-31
  orr r2, r2, r1
  str r2, [r0, #EXTI_IMR1]
  li r0, NVIC_ISER1
  movs r1, #(1 << 8)                       @ IRQ 40 = EXTI15_10
  str r1, [r0]
.global ready
ready:
loop:
  wfi
  b loop

.thumb_func
exti15_h:
  li r2, 0x20000200
  ldr r3, [r2]
  adds r3, r3, #1
  str r3, [r2]
  li r0, EXTI
  li r1, 1 << 13
  str r1, [r0, #EXTI_PR1]                  @ clear pending (write 1)
  li r0, GPIOB
  movs r1, #1
  str r1, [r0, #GPIO_BSRR]                 @ LD1 on
  li r0, GPIOE
  movs r1, #2
  str r1, [r0, #GPIO_BSRR]                 @ LD2 on
  bx lr

default_handler
