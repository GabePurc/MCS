# Synchronous exceptions: cause / epc / tval / mstatus recorded by the handler, mret resumes after
# the faulting instruction (or at the address in mscratch for fetch faults).
.option norvc

trap_handler:
    csrr t3, mcause
    REC t3
    csrr t3, mepc
    REC t3
    csrr t4, mtval
    REC t4
    csrr t4, mstatus
    REC t4
    csrr t4, mscratch
    bnez t4, 3f
    lhu t5, 0(t3)                 # length of the faulting instruction
    andi t5, t5, 3
    li t6, 3
    addi t3, t3, 2
    bne t5, t6, 2f
    addi t3, t3, 2
2:
    csrw mepc, t3
    mret
3:
    csrw mepc, t4                 # fetch fault: resume at the recovery address
    csrw mscratch, x0
    mret

_start:
    PROLOGUE
    la t0, trap_handler
    csrw mtvec, t0
    csrsi mstatus, 8              # MIE = 1: MPIE is 1 in the handler
    lui s0, 0x3fc80
    addi s0, s0, 0x400
    li a0, 0x1234
site_unimp:
    unimp                         # illegal (32-bit): tval = 0xc0001073
site_ecall:
    ecall
site_ebreak:
    ebreak
site_ldmis:
    lw a1, 1(s0)
site_ldmis2:
    lh a1, 3(s0)
site_stmis:
    sw a0, 2(s0)
site_stmis2:
    sh a0, 1(s0)
    li s1, 0x70000000
site_ldfault0:
    lw a1, 0(x0)
site_ldfault1:
    lbu a1, 0x10(s1)
    la t0, _start
site_stfault_rom:
    sw a0, 0(t0)                  # the IROM window is read-only
site_stfault_rom2:
    sb a0, 0(t0)
    lui t0, 0x3c000
site_stfault_drom:
    sw a0, 0(t0)                  # DROM is read-only too
site_stfault_unmapped:
    sw a0, 0(s1)
site_csr_ro:
    csrw mvendorid, a0            # write to a read-only CSR: tval = 0xf1151073
site_csr_ro2:
    csrrsi a2, mhartid, 1         # csrrsi with zimm != 0 writes: read-only CSR
site_csr_unknown:
    csrr a2, 0x7c0                # unimplemented CSR: tval = 0x7c002673
    # ---- legal accesses do not trap ----
    csrr a2, mvendorid
    REC a2
    # ---- instruction access faults (resume through mscratch) ----
    la t0, recover1
    csrw mscratch, t0
    lui t1, 0x3fc80
site_fetch_dram:
    jr t1                         # DRAM is not executable
recover1:
    la t0, recover2
    csrw mscratch, t0
    li t1, 0x10000000
site_fetch_unmapped:
    jalr ra, 0(t1)                # unmapped
recover2:
    la t0, recover3
    csrw mscratch, t0
    lui t1, 0x60000
site_fetch_mmio:
    jr t1                         # peripheral window
recover3:
    la t0, recover4
    csrw mscratch, t0
site_fetch_zero:
    jr zero
recover4:
    # ---- a 16-bit illegal instruction (all zero) ----
site_c_unimp:
    .2byte 0
    # ---- mret restored MIE ----
    csrr t1, mstatus
    REC t1                        # 0x1888
done:
    wfi
