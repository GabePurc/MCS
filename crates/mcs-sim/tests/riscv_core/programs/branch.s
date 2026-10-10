# Branches (all six conditions on a table of operand pairs), jal / jalr link and target handling.
.option norvc

# t5 |= bit when "op a0, a1" is taken
.macro BR op, bit
    \op a0, a1, 1f
    j 2f
1:
    ori t5, t5, \bit
2:
.endm

_start:
    PROLOGUE
    # ---- 1. branch conditions over operand pairs; one mask word per pair ----
    la s0, pairs
    li s1, 12
    li s2, 0
10:
    slli t0, s2, 3
    add t0, t0, s0
    lw a0, 0(t0)
    lw a1, 4(t0)
    li t5, 0
    BR beq, 1
    BR bne, 2
    BR blt, 4
    BR bge, 8
    BR bltu, 16
    BR bgeu, 32
    REC t5
    addi s2, s2, 1
    blt s2, s1, 10b
    # ---- 2. jal: link register, forward and backward jumps ----
j1:
    jal ra, f1
    la t0, j1
    sub t0, ra, t0               # ra - j1 == 4
    REC t0
    j 1f
    REC x0                       # skipped
1:
    jal x0, 2f
    REC x0                       # skipped
2:
    li t0, 0
    li t1, 10
3:                               # backward branch loop: sum 1..10
    add t0, t0, t1
    addi t1, t1, -1
    bnez t1, 3b
    REC t0
    # ---- 3. jalr: odd target bit is cleared, rd == rs1, offsets ----
    la t0, f2
    addi t0, t0, 1
j2:
    jalr ra, 0(t0)               # lands on f2 (bit 0 cleared)
    la t1, j2
    sub t1, ra, t1               # == 4
    REC t1
    la t0, f3
j3:
    jalr t0, 0(t0)               # link overwrites the base register after it was read
    la t1, j3
    sub t1, t0, t1               # == 4
    REC t1
    la t0, f4
    addi t0, t0, -8
    jalr ra, 8(t0)               # offset added to the base
    REC a0                       # set by f4
    la t0, f5
    jalr x0, 0(t0)               # tail jump without link, f5 jumps to done
done:
    ebreak

f1:
    ret
f2:
    ret
f3:
    jr t0
f4:
    li a0, 0x1234
    ret
f5:
    li a0, 0x5678
    REC a0
    j done

.balign 4
pairs:
    .word 0, 0
    .word 1, 0
    .word 0, 1
    .word 0xffffffff, 1
    .word 1, 0xffffffff
    .word 0x80000000, 0x7fffffff
    .word 0x7fffffff, 0x80000000
    .word 0xffffffff, 0xffffffff
    .word 0x80000000, 0x80000000
    .word 5, 7
    .word 0xfffffffe, 0xffffffff
    .word 0x80000000, 1
