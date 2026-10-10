@ Shared definitions of the STM32H743 test programs (assembled by gen_programs.py with Apple clang,
@ --target=thumbv7em-none-eabihf -mcpu=cortex-m7 -mfpu=fpv5-d16). Programs run from flash at
@ 0x0800_0000 (vector table first), keep results in DTCM and stop with BKPT. Register addresses
@ follow RM0433 / the CMSIS header stm32h743xx.h.
.syntax unified
.thumb

.equ SRAM_TOP,   0x20020000     @ DTCM: 128 KiB
.equ RCC,        0x58024400
.equ RCC_CR,     0x00
.equ RCC_CFGR,   0x10
.equ RCC_D1CFGR, 0x18
.equ RCC_D2CFGR, 0x1c
.equ RCC_D3CFGR, 0x20
.equ RCC_PLLCKSELR, 0x28
.equ RCC_PLLCFGR,0x2c
.equ RCC_PLL1DIVR,0x30
.equ RCC_AHB4ENR,0xe0
.equ RCC_APB1LENR,0xe8
.equ RCC_APB2ENR,0xf0
.equ RCC_APB4ENR,0xf4
.equ PWR,        0x58024800
.equ PWR_CSR1,   0x04
.equ PWR_CR3,    0x0c
.equ PWR_D3CR,   0x18
.equ FLASH_ACR,  0x52002000
.equ SYSCFG,     0x58000400
.equ SYSCFG_EXTICR1, 0x08
.equ SYSCFG_PWRCR, 0x2c
.equ EXTI,       0x58000000
.equ EXTI_RTSR1, 0x00
.equ EXTI_FTSR1, 0x04
.equ EXTI_IMR1,  0x80
.equ EXTI_PR1,   0x88
.equ GPIOB,      0x58020400
.equ GPIOC,      0x58020800
.equ GPIOD,      0x58020c00
.equ GPIOE,      0x58021000
.equ GPIO_MODER, 0x00
.equ GPIO_IDR,   0x10
.equ GPIO_ODR,   0x14
.equ GPIO_BSRR,  0x18
.equ GPIO_AFRL,  0x20
.equ GPIO_AFRH,  0x24
.equ NVIC_ISER0, 0xe000e100
.equ NVIC_ISER1, 0xe000e104
.equ USART3,     0x40004800
.equ TIM2,       0x40000000
.equ SYST,       0xe000e010
.equ CPACR,      0xe000ed88

@ Loads a 32-bit constant.
.macro li reg, val
  movw \reg, #(\val) & 0xffff
  movt \reg, #((\val) >> 16) & 0xffff
.endm

@ Global Thumb function.
.macro func name
  .global \name
  .thumb_func
\name:
.endm

@ Vector table with the handlers the programs use; everything else goes to `dflt`.
@ Core: reset (1), systick (15); IRQs: tim2 (28), usart3 (39), exti15_10 (40).
.macro vectors reset, systick=dflt, tim2=dflt, usart3=dflt, exti15=dflt
vbase:
  .word SRAM_TOP
  .word (\reset - vbase) + 0x08000000
  .rept 13
  .word (dflt - vbase) + 0x08000000
  .endr
  .word (\systick - vbase) + 0x08000000     @ 15
  .rept 28
  .word (dflt - vbase) + 0x08000000         @ IRQ 0-27
  .endr
  .word (\tim2 - vbase) + 0x08000000        @ IRQ 28
  .rept 10
  .word (dflt - vbase) + 0x08000000         @ IRQ 29-38
  .endr
  .word (\usart3 - vbase) + 0x08000000      @ IRQ 39
  .word (\exti15 - vbase) + 0x08000000      @ IRQ 40
  .rept 4
  .word (dflt - vbase) + 0x08000000
  .endr
.endm

@ Unexpected exception: stop with a recognisable BKPT.
.macro default_handler
.thumb_func
dflt:
  bkpt #0xff
  b dflt
.endm
