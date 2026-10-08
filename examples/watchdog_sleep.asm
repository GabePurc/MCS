; =============================================================================
;  watchdog_sleep.asm - the watchdog interrupt wakes the CPU from power-down
;  every ~0.5 s and toggles PB0. Shows CCP-protected register writes and
;  deep sleep (the simulator fast-forwards while the core sleeps).
;  Target: ATtiny10
; =============================================================================
.include "tn10def.inc"

.def    temp    = r16

.cseg
.org 0x0000
        rjmp    reset
.org WDTaddr
        rjmp    wdt_isr

reset:
        sbi     DDRB, DDB0

        ldi     temp, 0xD8                              ; unlock protected I/O
        out     CCP, temp                               ; for the next 4 cycles
        ldi     temp, (1<<WDIE) | (1<<WDP2) | (1<<WDP0) ; interrupt mode, 0.5 s
        out     WDTCSR, temp

        ldi     temp, (1<<SM1) | (1<<SE)                ; power-down sleep
        out     SMCR, temp
        sei

main:
        sleep
        rjmp    main

wdt_isr:
        sbi     PINB, PINB0
        reti
