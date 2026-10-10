# Bundled example (examples/esp32c3_blink.elf): blinks GPIO2 every 250 ms.
# GPIO8 is the RGB LED data pin of the ESP32-C3-DevKitM-1, so a plain GPIO is used instead.
# The CPU runs from the 40 MHz crystal divided by two (20 MHz) after reset: 4 cycles per delay iteration.
.equ GPIO, 0x60004000

.section .text.start,"ax"
.globl _start
_start:
    li s0, GPIO
    li t0, 1 << 2
    sw t0, 0x24(s0)                 # GPIO_ENABLE_W1TS: GPIO2 is an output
1:  sw t0, 0x08(s0)                 # GPIO_OUT_W1TS: GPIO2 high
    jal ra, delay
    sw t0, 0x0c(s0)                 # GPIO_OUT_W1TC: GPIO2 low
    jal ra, delay
    j 1b

# About 250 ms: 1_250_000 iterations x 4 cycles (addi 1 + taken bnez 3) at 20 MHz.
delay:
    li a0, 1250000
2:  addi a0, a0, -1
    bnez a0, 2b
    ret
