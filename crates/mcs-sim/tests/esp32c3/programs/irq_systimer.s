# SYSTIMER one-shot alarm (target mode) and then a periodic alarm, through the interrupt matrix
# (source 37 -> CPU interrupt 10) into a vectored mtvec handler.
.balign 256
vtable:
    .rept 10
    j bad
    .endr
    j isr10                         # CPU interrupt 10
    .rept 21
    j bad
    .endr

bad:
    ebreak

_start:
    PROLOGUE
    la t0, vtable
    ori t0, t0, 1
    csrw mtvec, t0
    li t0, INTC
    li t1, 10
    sw t1, 0x94(t0)                 # SYSTIMER_TARGET0 (37) -> CPU interrupt 10
    li t1, 1 << 10
    sw t1, 0x104(t0)
    li t1, 5
    sw t1, 0x114 + 4*10(t0)
    li t1, 1
    sw t1, 0x194(t0)                # threshold 1
    # one-shot: now + 800 ticks (50 us)
    jal ra, read_ticks
    mv s1, a0
    addi a0, a0, 800
    li t2, SYSTIMER
    sw a0, 0x20(t2)                 # TARGET0_LO
    sw zero, 0x1c(t2)               # TARGET0_HI
    sw zero, 0x34(t2)               # TARGET0_CONF: target mode, unit 0
    lw t3, 0(t2)
    lui t4, 0x1000                  # TARGET0_WORK_EN (bit 24)
    or t3, t3, t4
    sw t3, 0(t2)
    li t1, 1
    sw t1, 0x64(t2)                 # INT_ENA
    sw t1, 0x50(t2)                 # COMP0_LOAD
    li t0, 8
    csrs mstatus, t0
    li s2, 0
1:  wfi
    li t0, 1
    bne s2, t0, 1b                  # until the first alarm was handled
    REC s1                          # start tick
    # periodic: every 160 ticks (10 us), three alarms
    li t2, SYSTIMER
    lui t1, 0x40000                 # PERIOD_MODE (bit 30)
    addi t1, t1, 160
    sw t1, 0x34(t2)
    li t1, 1
    sw t1, 0x50(t2)                 # COMP0_LOAD: first alarm one period from now
2:  wfi
    li t0, 4
    bne s2, t0, 2b
    li t2, SYSTIMER
    sw zero, 0x64(t2)               # INT_ENA off
    ebreak

isr10:
    li t0, SYSTIMER
    li t1, 1
    sw t1, 0x6c(t0)                 # INT_CLR
    csrr t1, mcause
    REC t1
    li t1, 1 << 30
    sw t1, 4(t0)
    lw t1, 0x44(t0)                 # tick at handler entry
    REC t1
    addi s2, s2, 1
    mret
