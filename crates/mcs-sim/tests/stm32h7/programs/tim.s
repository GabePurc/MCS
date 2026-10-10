.include "common.inc"
vectors reset=reset, tim2=tim2_h

@ TIM2 (32-bit) update interrupt every 1 ms: PSC = 63, ARR = 999 on the 64 MHz HSI-driven timer clock;
@ the handler clears UIF and stops with BKPT.
func reset
  li r0, RCC
  movs r1, #1                              @ TIM2EN
  str r1, [r0, #RCC_APB1LENR]
  ldr r1, [r0, #RCC_APB1LENR]
  li r0, TIM2
  movs r1, #63
  str r1, [r0, #0x28]                      @ PSC
  li r1, 999
  str r1, [r0, #0x2c]                      @ ARR
  movs r1, #1
  str r1, [r0, #0x14]                      @ EGR.UG
  movs r1, #0
  str r1, [r0, #0x10]                      @ clear UIF
  movs r1, #1
  str r1, [r0, #0x0c]                      @ DIER.UIE
  li r0, NVIC_ISER0
  movs r1, #(1 << 28)                      @ IRQ 28 = TIM2
  str r1, [r0]
  li r0, TIM2
  movs r1, #1
  str r1, [r0]                             @ CR1.CEN
.global tim2_run
tim2_run:
  wfi
  b tim2_run

.thumb_func
tim2_h:
  li r0, TIM2
  movs r1, #0
  str r1, [r0, #0x10]                      @ clear UIF
  bkpt #0
  bx lr

default_handler
