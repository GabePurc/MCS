# Custom / optional CSRs of the ESP32-C3 hart: PMP (stored), performance counters (read 0, writes ignored),
# an unknown CSR traps as an illegal instruction.
_start:
    PROLOGUE
    la t0, handler
    csrw mtvec, t0
    li t0, 0x12345678
    csrw 0x3b0, t0                  # pmpaddr0
    csrr t1, 0x3b0
    REC t1
    li t0, 0x1f
    csrw 0x3a0, t0                  # pmpcfg0
    csrr t1, 0x3a0
    REC t1
    li t0, 0xff
    csrw 0x7e0, t0                  # mpcer: ignored
    csrr t1, 0x7e0
    REC t1
    csrr t1, 0x7e2                  # mpccr
    REC t1
    csrr t1, 0xf14                  # mhartid
    REC t1
    csrr t1, 0xf11                  # mvendorid
    REC t1
    csrr t1, 0x7ff                  # not implemented: illegal instruction
    li t1, 0x600d
    REC t1                          # reached after the handler skipped the instruction
    ebreak

handler:
    csrr t2, mcause
    REC t2
    csrr t2, mepc
    addi t2, t2, 4
    csrw mepc, t2
    mret
