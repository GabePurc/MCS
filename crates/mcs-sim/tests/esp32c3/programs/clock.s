# Clock switch: the same busy loop is timed (SYSTIMER ticks, 16 MHz) at the reset clock (XTAL/2 = 20 MHz)
# and after switching the CPU to the PLL at 160 MHz. GPIO2 toggles around each phase.
_start:
    PROLOGUE
    li s0, GPIO
    li t0, 4
    sw t0, 0x24(s0)                 # GPIO_ENABLE_W1TS: GPIO2 output
    sw t0, 0x08(s0)                 # OUT_W1TS: high
    jal ra, read_ticks
    mv s1, a0
    li a0, 1000
    jal ra, delay
    jal ra, read_ticks
    sub t2, a0, s1
    REC t2                          # [0] ticks for 1000 iterations at 20 MHz
    li t0, 4
    sw t0, 0x0c(s0)                 # OUT_W1TC: low
    # SOC_CLK_SEL = PLL (1), CPUPERIOD_SEL = 1 (160 MHz)
    li t3, SYSTEM
    li t0, 0x0d
    sw t0, 0x08(t3)                 # CPU_PER_CONF
    li t0, 0x401
    sw t0, 0x58(t3)                 # SYSCLK_CONF
    li t0, 4
    sw t0, 0x08(s0)                 # OUT_W1TS: high
    jal ra, read_ticks
    mv s1, a0
    li a0, 1000
    jal ra, delay
    jal ra, read_ticks
    sub t2, a0, s1
    REC t2                          # [1] ticks for 1000 iterations at 160 MHz
    li t0, 4
    sw t0, 0x0c(s0)                 # low
    ebreak
