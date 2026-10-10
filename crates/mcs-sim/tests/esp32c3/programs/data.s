# Initialised data in DRAM (.data is a loadable segment at 0x3FC8_0000): read it, change it.
_start:
    PROLOGUE
    la t0, val
    lw t1, 0(t0)
    REC t1
    li t2, 0x1badb002
    sw t2, 0(t0)
    lw t1, 0(t0)
    REC t1
    ebreak

.section .data
val: .word 0x12345678
