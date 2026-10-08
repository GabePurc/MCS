; =============================================================================
;  t85_pwm.asm - a potentiometer on PB2 (ADC1) sets the brightness of an LED on
;  PB1 (OC1A). Timer1 runs from the 64 MHz PLL / 16: 8-bit PWM at 15.6 kHz.
;  Target: ATtiny85 @ 1 MHz (factory fuses)
;  Try it: Pins & Stimulus > PB2 > "~" and move the voltage slider.
; =============================================================================
.include "tn85def.inc"

.org 0x0000
        rjmp    reset

reset:
        ldi     r16, high(RAMEND)
        out     SPH, r16
        ldi     r16, low(RAMEND)
        out     SPL, r16

        sbi     DDRB, DDB1              ; PB1 = OC1A output

        ; Start the PLL, wait until it locks, then clock Timer1 from it (PCK = 64 MHz)
        ldi     r16, (1<<PLLE)
        out     PLLCSR, r16
wait_lock:
        in      r16, PLLCSR
        sbrs    r16, PLOCK
        rjmp    wait_lock
        ldi     r16, (1<<PLLE)|(1<<PCKE)
        out     PLLCSR, r16

        ldi     r16, 255
        out     OCR1C, r16              ; TOP: 256 steps
        ldi     r16, (1<<PWM1A)|(1<<COM1A1)|(1<<CS12)|(1<<CS10)
        out     TCCR1, r16              ; PWM on OC1A, PCK / 16

        ; ADC1 (PB2), VCC reference, left adjusted (8-bit result in ADCH), free running
        ldi     r16, (1<<ADLAR)|1
        out     ADMUX, r16
        ldi     r16, (1<<ADEN)|(1<<ADSC)|(1<<ADATE)|(1<<ADPS1)|(1<<ADPS0)
        out     ADCSRA, r16

loop:
        in      r16, ADCH
        out     OCR1A, r16              ; duty cycle = pot position
        rjmp    loop
