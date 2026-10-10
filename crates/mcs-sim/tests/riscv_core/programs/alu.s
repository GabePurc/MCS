# RV32I integer ALU: every register-register op on all pairs of edge values, every immediate op
# on all edge values with edge immediates, lui/auipc and the hard-wired zero register.
.option norvc

.macro RR op
    \op t2, a0, a1
    REC t2
.endm

.macro RI op, imm
    \op t2, a0, \imm
    REC t2
.endm

_start:
    PROLOGUE
    la s0, edges
    li s1, 15
    # ---- 1. register-register over all pairs (10 results per pair) ----
    li s2, 0
1:
    slli t0, s2, 2
    add t0, t0, s0
    lw a0, 0(t0)
    li s3, 0
2:
    slli t1, s3, 2
    add t1, t1, s0
    lw a1, 0(t1)
    RR add
    RR sub
    RR sll
    RR slt
    RR sltu
    RR xor
    RR srl
    RR sra
    RR or
    RR and
    addi s3, s3, 1
    blt s3, s1, 2b
    addi s2, s2, 1
    blt s2, s1, 1b
    # ---- 2. immediates over all edge values (33 results per value) ----
    li s2, 0
3:
    slli t0, s2, 2
    add t0, t0, s0
    lw a0, 0(t0)
    RI addi, -2048
    RI addi, -1
    RI addi, 0
    RI addi, 1
    RI addi, 2047
    RI slti, -2048
    RI slti, -1
    RI slti, 0
    RI slti, 1
    RI slti, 2047
    RI sltiu, -2048
    RI sltiu, -1
    RI sltiu, 0
    RI sltiu, 1
    RI sltiu, 2047
    RI xori, -1
    RI xori, 0x7ff
    RI xori, -2048
    RI ori, -1
    RI ori, 0x7ff
    RI ori, -2048
    RI andi, -1
    RI andi, 0x7ff
    RI andi, -2048
    RI slli, 0
    RI slli, 1
    RI slli, 31
    RI srli, 0
    RI srli, 1
    RI srli, 31
    RI srai, 0
    RI srai, 1
    RI srai, 31
    addi s2, s2, 1
    blt s2, s1, 3b
    # ---- 3. lui / auipc ----
    lui t0, 0xfffff
    REC t0
    lui t0, 0x12345
    addi t0, t0, 0x678
    REC t0
    lui t0, 0x80000
    REC t0
    lui t0, 0
    REC t0
a1_:
    auipc t0, 0
    la t1, a1_
    sub t0, t0, t1
    REC t0
a2_:
    auipc t0, 1
    la t1, a2_
    sub t0, t0, t1
    REC t0
a3_:
    auipc t0, 0xfffff
    la t1, a3_
    sub t0, t0, t1
    REC t0
    # ---- 4. x0 is hard-wired to zero ----
    li t0, 7
    addi x0, x0, 5
    REC x0
    lui x0, 0x12345
    REC x0
    add x0, t0, t0
    REC x0
    lw x0, 0(s0)
    REC x0
    mul x0, t0, t0
    REC x0
    jal x0, 4f
4:
    REC x0
    csrr x0, mhartid
    REC x0
    add t1, x0, t0
    REC t1
    sub t1, x0, t0
    REC t1
    sltu t1, x0, t0
    REC t1
    # a nonzero value "written" to x0 never shows up as a source
    li x0, 0x3fc80000
    add t1, x0, x0
    REC t1
done:
    ebreak

.balign 4
edges:
    .word 0, 1, 0xffffffff, 2, 7, 31, 32, 33
    .word 0x7fffffff, 0x80000000, 0x80000001, 0x12345678, 0xdeadbeef, 100000, 0xfffffff9
