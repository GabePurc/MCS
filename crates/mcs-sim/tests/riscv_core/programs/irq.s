# Interrupts: vectored and direct mtvec, arbitration, masking, wfi. The test device at 0x6000_0000
# raises (offset 0) / lowers (offset 4) interrupt lines from the program.
.option norvc

.equ DEV, 0x60000000

# Handler body shared by all vectors. Records (mcause, ra, mepc), acknowledges interrupt causes
# by lowering their line, skips the instruction for exceptions.
common_handler:
    csrr t3, mcause
    REC t3
    REC ra
    csrr t4, mepc
    REC t4
    bltz t3, 1f
    addi t4, t4, 4                # exception (ecall): resume after it
    csrw mepc, t4
    mret
1:
    andi t3, t3, 31
    li t4, 1
    sll t4, t4, t3
    lui t5, 0x60000
    sw t4, 4(t5)                  # lower the line
    mret

.balign 256
vec_table:
    .rept 32
    jal ra, common_handler
    .endr

# ---- vectored mode ----
t_vectored:
    PROLOGUE
    la t0, vec_table
    ori t0, t0, 1
    csrw mtvec, t0
    li t0, 0x100000 | 0x20 | 0x800 | 0x8 | 0x80 | 0x200
    csrw mie, t0                  # enabled: 3, 5, 7, 9, 11, 20
    lui s0, 0x60000
    csrsi mstatus, 8
    # two lines at once: 20 (higher number) first, then 5
    li t1, 0x100000 | 0x20
v1:
    sw t1, 0(s0)
v1_after:
    nop
    # priority: MEI(11) > MSI(3) > MTI(7) > other lines (9)
    li t1, 0x800 | 0x8 | 0x80 | 0x200
v2:
    sw t1, 0(s0)
v2_after:
    nop
    # exception in vectored mode goes to the base (entry 0), not to base + 4 * cause
v_ecall:
    ecall
    # a line that is not enabled in mie stays pending ...
    li t1, 0x40                   # line 6
    sw t1, 0(s0)
    nop
    nop
    csrr t1, mip
    REC t1                        # 0x40
    # ... and is taken right after it gets enabled
    li t1, 0x40
v3:
    csrs mie, t1
v3_after:
    nop
    # MIE = 0 holds interrupts back; they are taken right after MIE is set again
    csrci mstatus, 8
    li t1, 0x20                   # line 5
    sw t1, 0(s0)
    nop
    nop
    csrr t1, mip
    REC t1                        # 0x20
v4:
    csrsi mstatus, 8
v4_after:
    nop
    csrr t1, mstatus
    REC t1                        # 0x1888
    csrr t1, mip
    REC t1                        # 0
done:
    ebreak

# ---- direct mode ----
t_direct:
    PROLOGUE
    la t0, common_handler
    csrw mtvec, t0
    li t0, 0x100000 | 0x2000      # lines 20 and 13
    csrw mie, t0
    lui s0, 0x60000
    csrsi mstatus, 8
    li ra, 0x1111
    li t1, 0x100000
d1:
    sw t1, 0(s0)                  # handler entered at the base: ra untouched
d1_after:
    nop
d_ecall:
    ecall
    li t1, 0x2000
d3:
    sw t1, 0(s0)
d3_after:
    nop
d_end:
    ebreak                        # halts the test (host breakpoint)

# ---- wfi ----
t_wfi:
    PROLOGUE
    la t0, common_handler
    csrw mtvec, t0
    li t0, 0x100000 | 0x20 | 0x40
    csrw mie, t0
    lui s0, 0x60000
    csrsi mstatus, 8
w1:
    wfi                           # sleeps until the host raises line 20
w1_after:
    li t1, 1
    REC t1                        # marker: executed after the handler returned
    csrci mstatus, 8              # MIE = 0: wfi still wakes on an enabled pending line
w2:
    wfi
w2_after:
    csrr t1, mip
    REC t1                        # 0x20 (host raised line 5, no trap taken)
    # a pending enabled line at wfi: no sleep
    li t1, 0x40
    sw t1, 0(s0)
w3:
    wfi
w3_after:
    li t1, 2
    REC t1
    csrsi mstatus, 8              # now the pending lines 5 and 6 are taken
w4_after:
    nop
done_wfi:
    ebreak

_start = t_vectored
