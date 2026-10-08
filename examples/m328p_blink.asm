; =============================================================================
;  m328p_blink.asm - blinks the Arduino Uno LED (PB5 = D13) twice a second
;  Target: ATmega328P @ 1 MHz (internal 8 MHz RC / 8, the factory fuses)
;  Timer1 in CTC mode raises a compare interrupt every 0.5 s; the CPU idles in
;  sleep between interrupts.
; =============================================================================
.include "m328Pdef.inc"

.org 0x0000
        jmp     reset                   ; 2-word vectors (JMP) on the ATmega328P
.org OC1Aaddr
        jmp     timer1_compa
.org INT_VECTORS_SIZE

reset:
        ldi     r16, high(RAMEND)
        out     SPH, r16
        ldi     r16, low(RAMEND)
        out     SPL, r16

        sbi     DDRB, DDB5              ; PB5 = output (LED)

        ; 1 MHz / 64 = 15625 timer ticks per second: compare match every 7812 ticks
        ldi     r16, high(7812 - 1)
        sts     OCR1AH, r16             ; 16-bit write: high byte first
        ldi     r16, low(7812 - 1)
        sts     OCR1AL, r16
        ldi     r16, (1<<OCIE1A)
        sts     TIMSK1, r16
        ldi     r16, (1<<WGM12)|(1<<CS11)|(1<<CS10)   ; CTC, clk/64
        sts     TCCR1B, r16

        ldi     r16, (1<<SE)            ; idle sleep between interrupts
        out     SMCR, r16
        sei
loop:
        sleep
        rjmp    loop

timer1_compa:
        sbi     PINB, PINB5             ; writing 1 to PINx toggles the pin
        reti
