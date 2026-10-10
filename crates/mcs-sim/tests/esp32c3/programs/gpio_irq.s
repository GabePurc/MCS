# GPIO interrupt: GPIO4 any-edge interrupt -> interrupt matrix source 16 -> CPU interrupt 12 (direct mtvec handler).
# The handler records mcause and GPIO_STATUS, clears the status and counts at RES - 4.
_start:
    PROLOGUE
    la t0, handler
    csrw mtvec, t0
    li t0, INTC
    li t1, 12
    sw t1, 0x40(t0)                 # GPIO source (16) -> CPU interrupt 12
    li t1, 1 << 12
    sw t1, 0x104(t0)                # CPU_INT_ENABLE
    li t1, 3
    sw t1, 0x114 + 4*12(t0)         # priority 3
    li t0, GPIO
    li t1, (3 << 7) | (1 << 13)     # PIN4: INT_TYPE = any edge, INT_ENA = CPU
    sw t1, 0x74 + 4*4(t0)
    li t0, 8
    csrs mstatus, t0                # MIE
1:  wfi
    j 1b

handler:
    csrr t0, mcause
    REC t0
    li t1, GPIO
    lw t0, 0x44(t1)                 # GPIO_STATUS
    REC t0
    sw t0, 0x4c(t1)                 # STATUS_W1TC
    li t1, RES - 4
    lw t0, 0(t1)
    addi t0, t0, 1
    sw t0, 0(t1)
    mret
