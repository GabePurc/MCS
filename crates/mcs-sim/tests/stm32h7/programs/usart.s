.include "common.inc"
vectors reset=tx_main

@ Nucleo-H743ZI virtual COM port: USART3 on PD8 (TX) / PD9 (RX), AF7, HSI 64 MHz, 115200 baud
@ (BRR = 64e6 / 115200 = 556 -> 556 cycles per bit).
func tx_main
  li r0, RCC
  movs r1, #(1 << 3)                       @ GPIODEN
  str r1, [r0, #RCC_AHB4ENR]
  li r1, 1 << 18                           @ USART3EN
  str r1, [r0, #RCC_APB1LENR]
  ldr r1, [r0, #RCC_APB1LENR]
  li r0, GPIOD
  li r1, 0xFFFAFFFF                        @ PD8, PD9 = alternate function
  str r1, [r0, #GPIO_MODER]
  li r1, 0x77
  str r1, [r0, #GPIO_AFRH]                 @ AF7 on PD8 / PD9
  li r0, USART3
  li r1, 556
  str r1, [r0, #0x0c]                      @ BRR
  movs r1, #0xd                            @ UE | RE | TE
  str r1, [r0, #0x00]
  movs r1, #'H'
  bl putc
  movs r1, #'i'
  bl putc
.global tx_sent
tx_sent:
  bkpt #0
1:
  ldr r1, [r0, #0x1c]
  tst r1, #(1 << 6)                        @ TC
  beq 1b
.global tx_done
tx_done:
  bkpt #1
  b tx_done

.thumb_func
putc:
2:
  ldr r2, [r0, #0x1c]
  tst r2, #(1 << 7)                        @ TXE
  beq 2b
  str r1, [r0, #0x28]                      @ TDR
  bx lr

default_handler
