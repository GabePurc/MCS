.include "common.inc"
vectors reset=reset, tim2=tim2_h

func reset
@ TIM2 update interrupt every 1000 x 16 cycles (PSC = 15, ARR = 999 at 16 MHz): 1 kHz.
@ The handler stops with BKPT at every update.
func tim2_main
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_APB1ENR1]     @ TIM2EN
  ldr r1, [r0, #RCC_APB1ENR1]
  li r0, TIM2
  movs r1, #15
  str r1, [r0, #0x28]             @ PSC
  li r1, 999
  str r1, [r0, #0x2c]             @ ARR
  movs r1, #1
  str r1, [r0, #0x14]             @ EGR: UG loads PSC / ARR
  movs r1, #0
  str r1, [r0, #0x10]             @ clear the UIF set by UG
  movs r1, #1
  str r1, [r0, #0x0c]             @ DIER: UIE
  li r2, NVIC_ISER0
  li r1, 1 << 28
  str r1, [r2]                    @ IRQ 28 = TIM2
  movs r1, #1
  str r1, [r0]                    @ CR1: CEN
.global tim2_run
tim2_run:
2:
  wfi
  b 2b

.thumb_func
tim2_h:
  li r0, TIM2
  movs r1, #0
  str r1, [r0, #0x10]             @ clear UIF
  bkpt #0
  bx lr

@ TIM3 channel 1 PWM mode 1 on PA6 (AF2): period 1000 cycles, duty 250.
func tim3_pwm
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_AHB2ENR]
  movs r1, #2
  str r1, [r0, #RCC_APB1ENR1]     @ TIM3EN
  ldr r1, [r0, #RCC_APB1ENR1]
  li r0, GPIOA
  li r1, 0xABFFEFFF               @ PA6 = alternate function
  str r1, [r0, #GPIO_MODER]
  li r1, 2 << 24                  @ AFRL: PA6 = AF2
  str r1, [r0, #GPIO_AFRL]
  li r0, TIM3
  movs r1, #0
  str r1, [r0, #0x28]             @ PSC = 0
  li r1, 999
  str r1, [r0, #0x2c]             @ ARR
  movs r1, #250
  str r1, [r0, #0x34]             @ CCR1
  movs r1, #0x68                  @ OC1M = PWM1 (110), OC1PE
  str r1, [r0, #0x18]             @ CCMR1
  movs r1, #1
  str r1, [r0, #0x20]             @ CCER: CC1E
  movs r1, #1
  str r1, [r0, #0x14]             @ EGR: UG
  movs r1, #0x81                  @ CR1: ARPE | CEN
  str r1, [r0]
.global pwm_run
pwm_run:
  b pwm_run

default_handler
