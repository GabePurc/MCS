# Cycle model: each measurement is (cycles between two mcycle reads) - 1 (the first csrr itself).
.option norvc

.macro T body
    csrr t0, mcycle
    \body
    csrr t1, mcycle
    sub t1, t1, t0
    addi t1, t1, -1
    REC t1
.endm

th:
    csrr t4, mcycle               # first instruction of the handler
    csrr t5, mepc
    addi t5, t5, 4
    csrw mepc, t5
    csrr t6, mcycle
    mret

_start:
    PROLOGUE
    lui s0, 0x3fc80
    addi s0, s0, 0x400
    li t2, 5
    li t3, 3
    T "add t2, t2, t2"            # 1
    T "lw t2, 0(s0)"              # 2
    T "sw t2, 0(s0)"              # 1
    T "lb t2, 0(s0)"              # 2
    T "mul t2, t2, t3"            # 1
    T "mulh t2, t2, t3"           # 1
    T "mulhsu t2, t2, t3"         # 1
    T "mulhu t2, t2, t3"          # 1
    li t2, 100
    T "div t2, t2, t3"            # 33
    li t2, 100
    T "divu t2, t2, t3"           # 33
    li t2, 100
    T "rem t2, t2, t3"            # 33
    li t2, 100
    T "remu t2, t2, t3"           # 33
    T "div t2, t2, x0"            # 33 (division by zero costs the same)
    T "lui t2, 5"                 # 1
    T "auipc t2, 5"               # 1
    T "csrr t2, mscratch"         # 1
    T "fence"                     # 1
    T "fence.i"                   # 3
    csrr t0, mcycle
    jal x0, 1f
1:
    csrr t1, mcycle
    sub t1, t1, t0
    addi t1, t1, -1
    REC t1                        # 2 (jal)
    csrr t0, mcycle
    beq x0, x0, 2f
2:
    csrr t1, mcycle
    sub t1, t1, t0
    addi t1, t1, -1
    REC t1                        # 3 (taken branch)
    csrr t0, mcycle
    bne x0, x0, 3f
3:
    csrr t1, mcycle
    sub t1, t1, t0
    addi t1, t1, -1
    REC t1                        # 1 (not-taken branch)
    la t5, 4f
    csrr t0, mcycle
    jalr x0, 0(t5)
4:
    csrr t1, mcycle
    sub t1, t1, t0
    addi t1, t1, -1
    REC t1                        # 3 (jalr)
    # trap entry and mret
    la t0, th
    csrw mtvec, t0
    csrr t0, mcycle
    ecall
    csrr t3, mcycle               # first instruction after mret
    sub t4, t4, t0
    addi t4, t4, -1
    REC t4                        # 3 (trap entry)
    sub t3, t3, t6
    addi t3, t3, -1
    REC t3                        # 3 (mret)
    # instret: loads/stores/branches all retire one instruction
    csrr t0, minstret
    lw t2, 0(s0)
    sw t2, 0(s0)
    beq x0, x0, 5f
5:
    mul t2, t2, t2
    csrr t1, minstret
    sub t1, t1, t0
    REC t1                        # 5
done:
    ebreak
