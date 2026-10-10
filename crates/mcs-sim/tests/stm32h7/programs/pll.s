.include "common.inc"
vectors reset=reset400, systick=systick_h

@ Clock bring-up following the HAL sequence: supply / voltage scaling, flash wait states, HSE (8 MHz),
@ PLL1 (DIVM1 = 1 -> 8 MHz reference, wide VCO), bus prescalers, SYSCLK = PLL1_P; then a 1 ms SysTick
@ (CPU clock) whose handler stops with BKPT each time.
@   reset400: VOS1, VCO 800 MHz / 2 = 400 MHz, AXI/AHB 200 MHz, APB 100 MHz, 2 wait states
@   reset480: VOS0 (VOS1 + SYSCFG overdrive), VCO 960 MHz / 2 = 480 MHz, AXI/AHB 240 MHz, APB 120 MHz, 4 wait states
@   reset_bad: 480 MHz without raising the voltage scaling or the wait states (must warn)
.macro bringup oden, acr, divn, tail
  li r0, PWR
1:
  ldr r1, [r0, #PWR_CSR1]
  tst r1, #(1 << 13)          @ ACTVOSRDY
  beq 1b
  .if \tail == 0
  li r1, 0xc000               @ VOS1
  str r1, [r0, #PWR_D3CR]
2:
  ldr r1, [r0, #PWR_D3CR]
  tst r1, #(1 << 13)          @ VOSRDY
  beq 2b
  .if \oden
  li r2, RCC
  ldr r1, [r2, #RCC_APB4ENR]
  orr r1, r1, #2              @ SYSCFGEN
  str r1, [r2, #RCC_APB4ENR]
  ldr r1, [r2, #RCC_APB4ENR]
  li r2, SYSCFG
  movs r1, #1                 @ ODEN
  str r1, [r2, #SYSCFG_PWRCR]
3:
  ldr r1, [r0, #PWR_D3CR]
  tst r1, #(1 << 13)
  beq 3b
  .endif
  li r2, FLASH_ACR
  li r1, \acr
  str r1, [r2]
  .endif
  li r0, RCC
  ldr r1, [r0, #RCC_CR]
  orr r1, r1, #(1 << 16)      @ HSEON
  str r1, [r0, #RCC_CR]
4:
  ldr r1, [r0, #RCC_CR]
  tst r1, #(1 << 17)          @ HSERDY
  beq 4b
  li r1, (2 << 0) | (1 << 4)  @ PLLSRC = HSE, DIVM1 = 1
  str r1, [r0, #RCC_PLLCKSELR]
  li r1, (3 << 2) | (1 << 16) @ PLL1RGE = 8-16 MHz, wide VCO, DIVP1EN
  str r1, [r0, #RCC_PLLCFGR]
  li r1, (\divn - 1) | (1 << 9) | (1 << 16) | (1 << 24)   @ N, P = 2, Q = 2, R = 2
  str r1, [r0, #RCC_PLL1DIVR]
  ldr r1, [r0, #RCC_CR]
  orr r1, r1, #(1 << 24)      @ PLL1ON
  str r1, [r0, #RCC_CR]
5:
  ldr r1, [r0, #RCC_CR]
  tst r1, #(1 << 25)          @ PLL1RDY
  beq 5b
  movs r1, #0x48              @ HPRE /2, D1PPRE /2
  str r1, [r0, #RCC_D1CFGR]
  li r1, 0x440                @ D2PPRE1 /2, D2PPRE2 /2
  str r1, [r0, #RCC_D2CFGR]
  movs r1, #0x40              @ D3PPRE /2
  str r1, [r0, #RCC_D3CFGR]
  ldr r1, [r0, #RCC_CFGR]
  orr r1, r1, #3              @ SW = PLL1
  str r1, [r0, #RCC_CFGR]
6:
  ldr r1, [r0, #RCC_CFGR]
  and r1, r1, #0x38
  cmp r1, #0x18               @ SWS = PLL1
  bne 6b
  li r2, 0x20000100
  ldr r1, [r0, #RCC_CFGR]
  str r1, [r2]
.endm

func reset400
  bringup 0, 0x22, 100, 0
  b .Lpll_done

func reset480
  bringup 1, 0x24, 120, 0
  b .Lpll_done

func reset_bad
  bringup 0, 0, 120, 1

.global pll_done
.Lpll_done:
pll_done:
  bkpt #0
  @ SysTick: reload for 1 ms of the CPU clock (cycles = Hz / 1000), core clock, interrupt on
  li r2, 0x20000104
  ldr r1, [r2]                @ reload value placed by the test (400 MHz: 399999, 480 MHz: 479999)
  li r0, SYST
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
