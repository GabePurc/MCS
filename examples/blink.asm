; =============================================================================
;  blink.asm - toggles an LED on PB0 roughly every 50 ms
;  Target: ATtiny10 @ 1 MHz (default: 8 MHz internal RC / 8)
; =============================================================================
.include "tn10def.inc"

.def    temp    = r16
.def    count1  = r17
.def    count2  = r18

.cseg
.org 0x0000
        rjmp    reset

reset:
        ldi     temp, high(RAMEND)      ; the stack pointer is set by hardware,
        out     SPH, temp               ; but initialising it is good practice
        ldi     temp, low(RAMEND)
        out     SPL, temp

        sbi     DDRB, DDB0              ; PB0 = output

loop:
        sbi     PINB, PINB0             ; writing 1 to PINx toggles the pin
        rcall   delay
        rjmp    loop

; -----------------------------------------------------------------------------
; delay: about 50 000 cycles (50 ms at 1 MHz)
; -----------------------------------------------------------------------------
delay:
        ldi     count2, 65
outer:
        ldi     count1, 255
inner:
        dec     count1                  ; 1 cycle
        brne    inner                   ; 2 cycles when taken
        dec     count2
        brne    outer
        ret
