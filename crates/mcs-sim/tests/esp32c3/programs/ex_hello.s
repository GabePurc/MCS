# Bundled example (examples/esp32c3_hello.elf): prints a greeting on UART0 (GPIO21 TX, GPIO20 RX, 115200 baud 8N1)
# and then echoes every received byte. Open View > Serial Monitor; it listens on GPIO21 by default.
.equ UART0, 0x60000000

.section .text.start,"ax"
.globl _start
_start:
    li s0, UART0
    li t0, 0x03700000               # CLK_CONF: XTAL clock source, SCLK_DIV_NUM = 0, clocks enabled
    sw t0, 0x78(s0)
    li t0, 347 | (3 << 20)          # CLKDIV = 347 + 3/16: 40 MHz / 347.1875 = 115213 baud
    sw t0, 0x14(s0)
    la s1, msg
1:  lbu t0, 0(s1)                   # send the zero-terminated greeting through the TX FIFO
    beqz t0, echo
    sw t0, 0(s0)
    addi s1, s1, 1
    j 1b
echo:
    lw t0, 0x1c(s0)                 # STATUS: RXFIFO_CNT in bits 9:0
    andi t0, t0, 0x3ff
    beqz t0, echo
    lw t0, 0(s0)                    # FIFO: take one byte ...
    sw t0, 0(s0)                    # ... and send it back
    j echo

.section .rodata
msg: .asciz "Hello from the ESP32-C3!\r\nType something and it will be echoed back.\r\n"
