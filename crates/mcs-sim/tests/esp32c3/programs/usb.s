# USB Serial/JTAG: prints a string stored in the DROM window (flash read through the data bus).
_start:
    PROLOGUE
    la s1, msg
    li s0, USB
1:  lbu t0, 0(s1)
    beqz t0, 2f
    sw t0, 0(s0)                    # EP1
    addi s1, s1, 1
    j 1b
2:  li t0, 1
    sw t0, 4(s0)                    # EP1_CONF.WR_DONE
    lw t0, 4(s0)
    REC t0                          # IN endpoint free
    ebreak

.section .rodata
msg: .asciz "Hello C3\n"
