; =============================================================================
;  button_interrupt.asm - a push button on PB2 (INT0, falling edge) toggles the
;  LED on PB0. The CPU sleeps (idle) between presses.
;  In the Pins window, drive PB2 Low/High (or click the pin) to "press" it.
;  Target: ATtiny10
; =============================================================================
.include "tn10def.inc"

.def    temp    = r16

.cseg
.org 0x0000
        rjmp    reset
.org INT0addr
        rjmp    int0_isr

reset:
        sbi     DDRB, DDB0              ; LED output
        sbi     PUEB, PUEB2             ; pull-up keeps PB2 high when released

        ldi     temp, (1<<ISC01)        ; INT0 on falling edge
        out     EICRA, temp
        ldi     temp, (1<<INT0)
        out     EIMSK, temp

        ldi     temp, (1<<SE)           ; sleep enable, idle mode
        out     SMCR, temp
        sei

main:
        sleep                           ; wait for an interrupt
        rjmp    main

int0_isr:
        sbi     PINB, PINB0             ; toggle the LED
        reti
