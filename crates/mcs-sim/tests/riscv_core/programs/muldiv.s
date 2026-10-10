# RV32M: mul / mulh / mulhsu / mulhu / div / divu / rem / remu on all pairs of edge values
# (includes division by zero and INT_MIN / -1).
.option norvc

_start:
    PROLOGUE
    la s0, edges
    li s1, 15
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
    mul t2, a0, a1
    REC t2
    mulh t2, a0, a1
    REC t2
    mulhsu t2, a0, a1
    REC t2
    mulhu t2, a0, a1
    REC t2
    div t2, a0, a1
    REC t2
    divu t2, a0, a1
    REC t2
    rem t2, a0, a1
    REC t2
    remu t2, a0, a1
    REC t2
    addi s3, s3, 1
    blt s3, s1, 2b
    addi s2, s2, 1
    blt s2, s1, 1b
    # destination equal to a source register
    li a0, 0x80000000
    li a1, -1
    div a0, a0, a1
    REC a0
    li a0, 0x80000000
    rem a0, a0, a1
    REC a0
    li a0, 123456789
    li a1, 1000
    mul a1, a0, a1
    REC a1
done:
    ebreak

.balign 4
edges:
    .word 0, 1, 0xffffffff, 2, 7, 31, 32, 33
    .word 0x7fffffff, 0x80000000, 0x80000001, 0x12345678, 0xdeadbeef, 100000, 0xfffffff9
