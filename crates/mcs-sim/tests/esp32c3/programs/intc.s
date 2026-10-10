# Interrupt controller: priorities, threshold and edge latching, using the four software interrupts
# (SYSTEM_CPU_INTR_FROM_CPU_n -> sources 50 - 53). Direct mtvec handler.
#   int 5 <- source 50, priority 3, level;   int 9 <- source 51, priority 7, level
#   int 11 <- source 52, priority 4, edge
_start:
    PROLOGUE
    la t0, handler
    csrw mtvec, t0
    li s0, INTC
    li t1, 5
    sw t1, 0xc8(s0)                 # source 50 -> 5
    li t1, 9
    sw t1, 0xcc(s0)                 # source 51 -> 9
    li t1, 11
    sw t1, 0xd0(s0)                 # source 52 -> 11
    li t1, 3
    sw t1, 0x114 + 4*5(s0)
    li t1, 7
    sw t1, 0x114 + 4*9(s0)
    li t1, 4
    sw t1, 0x114 + 4*11(s0)
    li t1, (1 << 5) | (1 << 9) | (1 << 11)
    sw t1, 0x104(s0)                # enable
    li t1, 1 << 11
    sw t1, 0x108(s0)                # int 11 is edge triggered
    li s1, SYSTEM
    # --- both level interrupts pending while MIE = 0, then enabled: 9 (priority 7) goes first
    li t1, 1
    sw t1, 0x28(s1)                 # int 5 source
    sw t1, 0x2c(s1)                 # int 9 source
    li t0, 8
    csrs mstatus, t0
    nop
    nop
    csrc mstatus, t0
    # --- threshold 4: priority 3 is masked, priority 7 is not
    li t1, 4
    sw t1, 0x194(s0)
    li t1, 1
    sw t1, 0x28(s1)
    sw t1, 0x2c(s1)
    csrs mstatus, t0
    nop
    nop
    csrc mstatus, t0
    lw t2, 0x110(s0)                # CPU_INT_EIP_STATUS: int 5 still pending
    REC t2
    sw zero, 0x194(s0)              # threshold 0: int 5 now served
    csrs mstatus, t0
    nop
    nop
    csrc mstatus, t0
    # --- edge interrupt: pulse the source (set, clear) before enabling interrupts; the latch stays
    li t1, 1
    sw t1, 0x30(s1)                 # source 52 high
    sw zero, 0x30(s1)               # low again
    lw t2, 0x110(s0)
    REC t2                          # int 11 latched
    csrs mstatus, t0
    nop
    nop
    csrc mstatus, t0
    lw t2, 0x110(s0)
    REC t2                          # cleared by the handler
    ebreak

handler:
    csrr t3, mcause
    REC t3
    andi t3, t3, 31
    li t4, 5
    beq t3, t4, 1f
    li t4, 9
    beq t3, t4, 2f
    # int 11 (edge): acknowledge in the controller
    li t4, 1 << 11
    sw t4, 0x10c(s0)
    sw zero, 0x10c(s0)
    mret
1:  sw zero, 0x28(s1)               # drop the level source of int 5
    mret
2:  sw zero, 0x2c(s1)
    mret
