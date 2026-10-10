# UART1 transmitting through the GPIO matrix: signal 9 (U1TXD) is routed to GPIO7 (FUNC7_OUT_SEL_CFG).
_start:
    PROLOGUE
    li s0, GPIO
    li t0, 9
    sw t0, 0x554 + 4*7(s0)          # OUT_SEL = U1TXD; the output enable comes from the peripheral
    li s1, 0x60010000               # UART1
    li t0, 0x03700000               # XTAL clock source, SCLK_DIV_NUM = 0
    sw t0, 0x78(s1)
    li t0, 347 | (3 << 20)
    sw t0, 0x14(s1)
    li t0, 'X'
    sw t0, 0(s1)
    li t0, 'Y'
    sw t0, 0(s1)
    li t1, 1 << 14
1:  lw t0, 4(s1)                    # TX_DONE
    and t0, t0, t1
    beqz t0, 1b
    lw t0, 0x1c(s1)                 # STATUS (TXD level high when idle)
    REC t0
    ebreak
