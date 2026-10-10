# UART0 at 115200 baud (XTAL clock source): sends "Hi\n" through the FIFO, then echoes the next received byte + 1.
_start:
    PROLOGUE
    li s0, UART0
    li t0, 0x03700000               # CLK_CONF: XTAL clock source, SCLK_DIV_NUM = 0 (reset value 1), clocks enabled
    sw t0, 0x78(s0)
    li t0, 347 | (3 << 20)          # CLKDIV = 347 + 3/16: 40 MHz / 347.1875 = 115213 baud
    sw t0, 0x14(s0)
    la s1, msg
1:  lbu t0, 0(s1)
    beqz t0, 2f
    sw t0, 0(s0)                    # FIFO
    addi s1, s1, 1
    j 1b
2:  li t1, 1 << 14                  # TX_DONE
3:  lw t0, 4(s0)
    and t0, t0, t1
    beqz t0, 3b
    sw t1, 0x10(s0)                 # INT_CLR
    lw t0, 0x1c(s0)                 # STATUS
    REC t0
4:  lw t0, 0x1c(s0)                 # wait for a received byte (RXFIFO_CNT)
    andi t0, t0, 0x3ff
    beqz t0, 4b
    lw t0, 0(s0)                    # read it
    REC t0
    addi t0, t0, 1
    sw t0, 0(s0)                    # echo + 1
5:  lw t0, 4(s0)
    and t0, t0, t1
    beqz t0, 5b
    ebreak

.section .rodata
msg: .asciz "Hi\n"
