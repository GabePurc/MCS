# GPIO output and input through the GPIO matrix: GPIO2 blinks with W1TS / W1TC, GPIO3 is read back (output
# readback needs the input enable of the pad, set at reset), GPIO5 is an input with its pull-up.
_start:
    PROLOGUE
    li s0, GPIO
    li t0, (1 << 2) | (1 << 3)
    sw t0, 0x24(s0)                 # ENABLE_W1TS: GPIO2, GPIO3 outputs
    li s2, 6                        # six full periods on GPIO2
1:  li t0, 4
    sw t0, 0x08(s0)                 # GPIO2 high
    li t0, 8
    sw t0, 0x08(s0)                 # GPIO3 high too
    lw t1, 0x3c(s0)                 # GPIO_IN
    REC t1
    li a0, 500
    jal ra, delay
    li t0, 12
    sw t0, 0x0c(s0)                 # both low
    li a0, 500
    jal ra, delay
    addi s2, s2, -1
    bnez s2, 1b
    lw t1, 0x3c(s0)                 # input bits: GPIO5 floats high (pull-up), outputs low
    REC t1
    lw t1, 0x04(s0)                 # GPIO_OUT
    REC t1
    lw t1, 0x20(s0)                 # GPIO_ENABLE
    REC t1
    ebreak
