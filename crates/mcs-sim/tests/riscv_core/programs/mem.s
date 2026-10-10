# Loads / stores of every width, sign and zero extension, alias windows (DRAM / IRAM / DROM).
.option norvc

_start:
    PROLOGUE
    lui s0, 0x3fc80
    addi s0, s0, 0x400          # s0 = 0x3FC80400 (DRAM scratch)
    # ---- 1. widths and extension ----
    li t0, 0x89abcdef
    sw t0, 0(s0)
    lw t1, 0(s0)
    REC t1
    lh t1, 0(s0)                 # 0xffffcdef
    REC t1
    lh t1, 2(s0)                 # 0xffff89ab
    REC t1
    lhu t1, 0(s0)
    REC t1
    lhu t1, 2(s0)
    REC t1
    lb t1, 0(s0)                 # 0xffffffef
    REC t1
    lb t1, 1(s0)                 # 0xffffffcd
    REC t1
    lb t1, 2(s0)
    REC t1
    lb t1, 3(s0)
    REC t1
    lbu t1, 0(s0)
    REC t1
    lbu t1, 1(s0)
    REC t1
    lbu t1, 2(s0)
    REC t1
    lbu t1, 3(s0)
    REC t1
    # ---- 2. sub-word stores only touch their own bytes ----
    sb t0, 1(s0)                 # byte 1 <- 0xef
    lw t1, 0(s0)
    REC t1
    li t2, 0x12345678
    sh t2, 2(s0)                 # halfword 1 <- 0x5678
    lw t1, 0(s0)
    REC t1
    sh t2, 0(s0)
    lw t1, 0(s0)
    REC t1
    sb t2, 3(s0)                 # byte 3 <- 0x78
    lw t1, 0(s0)
    REC t1
    # ---- 3. negative / extreme offsets ----
    addi s1, s0, 1024
    li t2, 0x0badf00d
    sw t2, -1024(s1)
    lw t1, 0(s0)
    REC t1
    sw t2, 2044(s0)
    addi s1, s0, 2044
    lw t1, 0(s1)
    REC t1
    addi s1, s0, 2047
    addi s1, s1, 1
    lw t1, -4(s1)
    REC t1
    # ---- 4. alias windows: DRAM 0x3FC80000 == IRAM 0x40380000, DROM 0x3C000000 == IROM ----
    li t2, 0xa5a55a5a
    sw t2, 0x500(s0)             # 0x3FC80900
    lui s1, 0x40380
    addi s1, s1, 0x400
    lw t1, 0x500(s1)             # same word through the IRAM window
    REC t1
    sb t2, 0x504(s1)             # store through the IRAM window
    lbu t1, 0x504(s0)
    REC t1
    la s1, edges
    lw t1, 4(s1)
    REC t1
    lui t3, 0x42000
    sub s1, s1, t3
    lui t3, 0x3c000
    add s1, s1, t3               # edges through the DROM window
    lw t1, 4(s1)
    REC t1
    lw t1, 12(s1)
    REC t1
    # ---- 5. store/load round trip through every register class ----
    li a0, 0xfeedface
    sw a0, 0x10(s0)
    lw a7, 0x10(s0)
    REC a7
    sw ra, 0x14(s0)
    sw sp, 0x18(s0)
    lw t1, 0x18(s0)
    sub t1, t1, sp
    REC t1
done:
    ebreak

.balign 4
edges:
    .word 0, 0x11223344, 2, 0x55667788
