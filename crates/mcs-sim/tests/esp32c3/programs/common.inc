# Common definitions and macros of the ESP32-C3 test programs.
.option norvc

.equ UART0,     0x60000000
.equ GPIO,      0x60004000
.equ IOMUX,     0x60009000
.equ SYSTIMER,  0x60023000
.equ TIMG0,     0x6001F000
.equ SYSTEM,    0x600C0000
.equ INTC,      0x600C2000
.equ USB,       0x60043000
.equ RES,       0x3FC80100          # results area (DRAM): REC appends words here

.section .text.start,"ax"
.globl _start

# Result pointer, stack pointer.
.macro PROLOGUE
    lui sp, 0x3fce0
    li s11, RES
.endm

.macro REC reg
    sw \reg, 0(s11)
    addi s11, s11, 4
.endm

# a0 = SYSTIMER unit 0 count (low word).
read_ticks:
    li t0, SYSTIMER
    li t1, 1 << 30
    sw t1, 4(t0)                    # UNIT0_OP.UPDATE
    lw a0, 0x44(t0)                 # UNIT0_VALUE_LO
    ret

# Busy loop of a0 iterations (4 cycles each: addi 1 + taken bnez 3).
delay:
1:  addi a0, a0, -1
    bnez a0, 1b
    ret
