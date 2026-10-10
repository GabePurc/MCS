.include "common.inc"
vectors reset=reset

func reset
@ USART1 on PA9 (AF7) at 115200 baud from HSI16 (BRR = 139): sends "Hi", waits for TC.
func tx_main
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_AHB2ENR]      @ GPIOAEN
  li r1, 1 << 14
  str r1, [r0, #RCC_APB2ENR]      @ USART1EN
  ldr r1, [r0, #RCC_APB2ENR]
  li r0, GPIOA
  li r1, 0xABFBFFFF               @ PA9 = alternate function (10)
  str r1, [r0, #GPIO_MODER]
  li r1, 7 << 4                   @ AFRH: PA9 = AF7
  str r1, [r0, #GPIO_AFRH]
  li r0, USART1
  movs r1, #139
  str r1, [r0, #0x0c]             @ BRR
  movs r1, #9                     @ UE | TE
  str r1, [r0]
  movs r4, #'H'
  bl putc
  movs r4, #'i'
  bl putc
.global tx_sent
tx_sent:
  bkpt #0
2:
  ldr r1, [r0, #0x1c]
  tst r1, #0x40                   @ TC
  beq 2b
.global tx_done
tx_done:
  bkpt #0
  b tx_done

.thumb_func
putc:
1:
  ldr r1, [r0, #0x1c]
  tst r1, #0x80                   @ TXE
  beq 1b
  str r4, [r0, #0x28]             @ TDR
  bx lr

@ Receives two bytes on PA10 (AF7) and stores them at 0x20000100.
func rx_main
  li r0, RCC
  movs r1, #1
  str r1, [r0, #RCC_AHB2ENR]
  li r1, 1 << 14
  str r1, [r0, #RCC_APB2ENR]
  ldr r1, [r0, #RCC_APB2ENR]
  li r0, GPIOA
  li r1, 0xABEFFFFF               @ PA10 = alternate function (10)
  str r1, [r0, #GPIO_MODER]
  li r1, 7 << 8                   @ AFRH: PA10 = AF7
  str r1, [r0, #GPIO_AFRH]
  li r0, USART1
  movs r1, #139
  str r1, [r0, #0x0c]
  movs r1, #5                     @ UE | RE
  str r1, [r0]
  li r4, 0x20000100
.global rx_ready
rx_ready:
  bkpt #0
3:
  ldr r1, [r0, #0x1c]
  tst r1, #0x20                   @ RXNE
  beq 3b
  ldr r2, [r0, #0x24]             @ RDR
  strb r2, [r4], #1
  b 3b

default_handler
