@ Shared macros of the ARM core test programs (assembled by gen_programs.py with Apple clang).
@ Programs run from flash at 0x0800_0000 (vector table first); results are stored through r12
@ (set to the start of SRAM + 0x100) with the `out` macro; success is a BKPT at `done`.
.syntax unified
.thumb

.macro out reg
  str \reg, [r12], #4
.endm

.macro setr12
  movw r12, #0x100
  movt r12, #0x2000
.endm

@ Loads a 32-bit constant.
.macro li reg, val
  movw \reg, #(\val) & 0xffff
  movt \reg, #((\val) >> 16) & 0xffff
.endm

.macro vectors reset, nmi=dflt, hard=dflt, mem=dflt, bus=dflt, usage=dflt, svc=dflt, dbg=dflt, pendsv=dflt, systick=dflt, irq0=dflt, irq1=dflt, irq2=dflt, irq3=dflt
vbase:
  .word 0x20008000
  .word (\reset - vbase) + 0x08000000
  .word (\nmi - vbase) + 0x08000000
  .word (\hard - vbase) + 0x08000000
  .word (\mem - vbase) + 0x08000000
  .word (\bus - vbase) + 0x08000000
  .word (\usage - vbase) + 0x08000000
  .word 0, 0, 0, 0
  .word (\svc - vbase) + 0x08000000
  .word (\dbg - vbase) + 0x08000000
  .word 0
  .word (\pendsv - vbase) + 0x08000000
  .word (\systick - vbase) + 0x08000000
  .word (\irq0 - vbase) + 0x08000000
  .word (\irq1 - vbase) + 0x08000000
  .word (\irq2 - vbase) + 0x08000000
  .word (\irq3 - vbase) + 0x08000000
.endm

.macro func name
  .global \name
  .thumb_func
\name:
.endm

@ Local function (branch targets must not be global symbols: the assembler would leave a
@ relocation for BL instead of resolving it).
.macro lfunc name
  .thumb_func
\name:
.endm
