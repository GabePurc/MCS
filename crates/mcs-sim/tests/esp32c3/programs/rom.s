# Calling into the boot ROM (not simulated) stops the simulation.
_start:
    PROLOGUE
    li a0, 42
    li t0, 0x40000100
    jalr ra, 0(t0)
    ebreak
