@ Shared definitions of the STM32G4 test programs (assembled by gen_programs.py with Apple clang,
@ --target=thumbv7em-none-eabi). Programs run from flash at 0x0800_0000 (vector table first), keep
@ results in SRAM and stop with BKPT. Register addresses follow RM0440 / the CMSIS header.
.syntax unified
.thumb

.equ SRAM_TOP,   0x20020000     @ STM32G474: 96 KiB SRAM1+2 + 32 KiB CCM
.equ RCC,        0x40021000
.equ RCC_CR,     0x00
.equ RCC_CFGR,   0x08
.equ RCC_PLLCFGR,0x0c
.equ RCC_AHB2ENR,0x4c
.equ RCC_APB1ENR1,0x58
.equ RCC_APB2ENR,0x60
.equ FLASH_ACR,  0x40022000
.equ PWR_CR5,    0x40007080
.equ GPIOA,      0x48000000
.equ GPIO_MODER, 0x00
.equ GPIO_PUPDR, 0x0c
.equ GPIO_IDR,   0x10
.equ GPIO_ODR,   0x14
.equ GPIO_BSRR,  0x18
.equ GPIO_AFRL,  0x20
.equ GPIO_AFRH,  0x24
.equ SYSCFG,     0x40010000
.equ EXTI,       0x40010400
.equ NVIC_ISER0, 0xe000e100
.equ USART1,     0x40013800
.equ TIM2,       0x40000000
.equ TIM3,       0x40000400
.equ SYST,       0xe000e010

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
@ Core: reset, systick; IRQs: exti0 (6), tim2 (28), tim3 (29), usart1 (37).
.macro vectors reset, systick=dflt, exti0=dflt, tim2=dflt, tim3=dflt, usart1=dflt
vbase:
  .word SRAM_TOP
  .word (\reset - vbase) + 0x08000000
  .rept 13
  .word (dflt - vbase) + 0x08000000
  .endr
  .word (\systick - vbase) + 0x08000000     @ 15
  .rept 6
  .word (dflt - vbase) + 0x08000000         @ IRQ 0-5
  .endr
  .word (\exti0 - vbase) + 0x08000000       @ IRQ 6
  .rept 21
  .word (dflt - vbase) + 0x08000000         @ IRQ 7-27
  .endr
  .word (\tim2 - vbase) + 0x08000000        @ IRQ 28
  .word (\tim3 - vbase) + 0x08000000        @ IRQ 29
  .rept 7
  .word (dflt - vbase) + 0x08000000         @ IRQ 30-36
  .endr
  .word (\usart1 - vbase) + 0x08000000      @ IRQ 37
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
