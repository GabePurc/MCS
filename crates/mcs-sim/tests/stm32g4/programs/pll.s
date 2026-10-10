.include "common.inc"
vectors reset=reset, systick=systick_h

@ HSI16 -> PLL (HSI16 / 4 * 85 / 2 = 170 MHz), Range 1 boost, 8 wait states; then a 1 ms SysTick
@ (HCLK) whose handler stops with BKPT each time.
func reset
  @ PWR clock, boost mode (R1MODE = 0)
  li r0, RCC
  ldr r1, [r0, #RCC_APB1ENR1]
  orr r1, r1, #(1 << 28)
  str r1, [r0, #RCC_APB1ENR1]
  li r2, PWR_CR5
  movs r1, #0
  str r1, [r2]
  @ Flash latency 8
  li r2, FLASH_ACR
  li r1, 0x00040608
  str r1, [r2]
  @ PLL: PLLSRC = HSI16 (2), PLLM = /4 (3), PLLN = 85, PLLREN, PLLR = /2 (0)
  li r1, (2 << 0) | (3 << 4) | (85 << 8) | (1 << 24)
  str r1, [r0, #RCC_PLLCFGR]
  ldr r1, [r0, #RCC_CR]
  orr r1, r1, #(1 << 24)
  str r1, [r0, #RCC_CR]
1:
  ldr r1, [r0, #RCC_CR]
  tst r1, #(1 << 25)
  beq 1b
  @ SW = PLL
  ldr r1, [r0, #RCC_CFGR]
  orr r1, r1, #3
  str r1, [r0, #RCC_CFGR]
2:
  ldr r1, [r0, #RCC_CFGR]
  and r1, r1, #0xc
  cmp r1, #0xc
  bne 2b
  li r2, 0x20000100
  ldr r1, [r0, #RCC_CFGR]
  str r1, [r2]
.global pll_done
pll_done:
  bkpt #0
  @ SysTick: 170000 cycles = 1 ms at 170 MHz, core clock, interrupt on
  li r0, SYST
  li r1, 169999
  str r1, [r0, #4]
  movs r1, #0
  str r1, [r0, #8]
  movs r1, #7
  str r1, [r0]
.global spin
spin:
  b spin

.thumb_func
systick_h:
  bkpt #0
  bx lr

default_handler
