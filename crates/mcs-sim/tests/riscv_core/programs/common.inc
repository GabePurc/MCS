.section .text.start,"ax"
.globl _start

# Result convention: s11 points into DRAM (0x3FC80100 on entry); REC appends a word.
.macro REC reg
    sw \reg, 0(s11)
    addi s11, s11, 4
.endm

# Standard prologue: result pointer, stack pointer.
.macro PROLOGUE
    lui s11, 0x3fc80
    addi s11, s11, 0x100
    lui sp, 0x3fc90
.endm
