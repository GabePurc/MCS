# Zicsr semantics and the machine-mode CSR file (values documented in cpu.rs).
.option norvc

_start:
    PROLOGUE
    # ---- 1. csrrw / csrrs / csrrc return the old value; immediate forms ----
    li t0, 0x12345678
    csrw mscratch, t0
    csrr t1, mscratch
    REC t1                        # 0x12345678
    li t2, 0xa5a5a5a5
    csrrw t1, mscratch, t2
    REC t1                        # 0x12345678 (old)
    csrr t1, mscratch
    REC t1                        # 0xa5a5a5a5
    li t2, 0x0f0f0f0f
    csrrs t1, mscratch, t2
    REC t1                        # 0xa5a5a5a5
    csrr t1, mscratch
    REC t1                        # 0xafafafaf
    li t2, 0xff00ff00
    csrrc t1, mscratch, t2
    REC t1                        # 0xafafafaf
    csrr t1, mscratch
    REC t1                        # 0x00af00af
    csrrwi t1, mscratch, 21
    REC t1                        # 0x00af00af
    csrr t1, mscratch
    REC t1                        # 21
    csrrsi t1, mscratch, 10
    REC t1                        # 21
    csrr t1, mscratch
    REC t1                        # 31
    csrrci t1, mscratch, 5
    REC t1                        # 31
    csrr t1, mscratch
    REC t1                        # 26
    csrrsi t1, mscratch, 0        # zimm = 0: read only
    REC t1                        # 26
    csrrci t1, mscratch, 0
    REC t1                        # 26
    csrrs t1, mscratch, x0        # rs1 = x0: read only
    REC t1                        # 26
    csrrc t1, mscratch, x0
    REC t1                        # 26
    # ---- 2. read-only identification CSRs (read-only accesses are legal) ----
    csrr t1, misa
    REC t1                        # 0x40001104
    csrr t1, mvendorid
    REC t1                        # 0x612
    csrr t1, marchid
    REC t1                        # 0x80000001
    csrr t1, mimpid
    REC t1                        # 1
    csrr t1, mhartid
    REC t1                        # 0
    csrrsi t1, mvendorid, 0
    REC t1                        # 0x612 (no write: no trap)
    csrw misa, x0                 # WARL: ignored
    csrr t1, misa
    REC t1                        # 0x40001104
    # ---- 3. mstatus ----
    csrw mstatus, x0
    csrr t1, mstatus
    REC t1                        # 0x1800 (MPP hardwired to M)
    csrsi mstatus, 8
    csrr t1, mstatus
    REC t1                        # 0x1808 (MIE)
    li t0, -1
    csrw mstatus, t0
    csrr t1, mstatus
    REC t1                        # 0x1888 (MIE | MPIE | MPP)
    csrci mstatus, 8
    csrr t1, mstatus
    REC t1                        # 0x1880
    csrw mstatus, x0
    # ---- 4. mie / mip / mtvec / mepc / mcause / mtval / mcounteren ----
    li t0, -1
    csrw mie, t0
    csrr t1, mie
    REC t1                        # 0xfffffffe (line 0 does not exist)
    csrw mie, x0
    csrw mip, t0                  # driven by hardware: ignored
    csrr t1, mip
    REC t1                        # 0
    li t0, 0x42000103
    csrw mtvec, t0
    csrr t1, mtvec
    REC t1                        # 0x42000101 (reserved mode bit cleared)
    li t0, 0x42000100
    csrw mtvec, t0
    csrr t1, mtvec
    REC t1                        # 0x42000100
    li t0, 0x42000102
    csrw mtvec, t0
    csrr t1, mtvec
    REC t1                        # 0x42000100
    csrw mtvec, x0
    li t0, 0x42000003
    csrw mepc, t0
    csrr t1, mepc
    REC t1                        # 0x42000002 (bit 0 cleared, bit 1 kept: C extension)
    li t0, 0x42000005
    csrw mepc, t0
    csrr t1, mepc
    REC t1                        # 0x42000004
    li t0, 0x8000000b
    csrw mcause, t0
    csrr t1, mcause
    REC t1                        # 0x8000000b
    li t0, 0xdeadbeef
    csrw mtval, t0
    csrr t1, mtval
    REC t1                        # 0xdeadbeef
    li t0, -1
    csrw mcounteren, t0
    csrr t1, mcounteren
    REC t1                        # 7
    csrw mcounteren, x0
    # ---- 5. counters ----
    csrr t0, mcycle
    nop
    nop
    nop
    nop
    csrr t1, mcycle
    sub t1, t1, t0
    REC t1                        # 5 = csrr + 4 nops
    csrr t0, minstret
    nop
    nop
    nop
    csrr t1, minstret
    sub t1, t1, t0
    REC t1                        # 4
    csrr t0, cycle
    csrr t1, mcycle
    sub t1, t1, t0
    REC t1                        # 1 (user alias)
    csrr t0, instret
    csrr t1, minstret
    sub t1, t1, t0
    REC t1                        # 1
    csrw mcycle, x0
    csrr t1, mcycle
    REC t1                        # 1
    csrw minstret, x0
    csrr t1, minstret
    REC t1                        # 1
    li t0, 0xfffffff0
    csrw mcycle, t0
    .rept 20
    nop
    .endr
    csrr t1, mcycle
    csrr t2, mcycleh
    REC t1                        # 5 (wrapped)
    REC t2                        # 1 (carry into the high word)
    li t0, 7
    csrw mcycleh, t0
    csrr t1, mcycleh
    REC t1                        # 7
    csrr t1, cycleh
    REC t1                        # 7
    li t0, 3
    csrw minstreth, t0
    csrr t1, minstreth
    REC t1                        # 3
    csrr t1, instreth
    REC t1                        # 3
    # ---- 6. hardware performance monitor CSRs read as zero, writes are ignored ----
    li t0, -1
    csrw mhpmcounter3, t0
    csrr t1, mhpmcounter3
    REC t1                        # 0
    csrr t1, hpmcounter31
    REC t1                        # 0
    csrw mhpmevent3, t0
    csrr t1, mhpmevent3
    REC t1                        # 0
    csrw mcountinhibit, t0
    csrr t1, mcountinhibit
    REC t1                        # 0
    csrr t1, mhpmcounter3h
    REC t1                        # 0
done:
    ebreak
