; =============================================================================
;  adc_to_pwm.asm - reads the voltage on PB2 (ADC2) and copies the 8-bit result
;  to the PWM duty cycle on PB0 (OC0A). Set PB2 to "Analog" in the Pins window
;  and move the voltage slider.
;  Target: ATtiny10 (ATtiny5/10 have the ADC)
; =============================================================================
.include "tn10def.inc"

.def    temp    = r16
.def    zero    = r17

.cseg
.org 0x0000
        rjmp    reset

reset:
        clr     zero
        sbi     DDRB, DDB0                              ; OC0A output

        ldi     temp, (1<<COM0A1) | (1<<WGM00)          ; fast PWM 8-bit
        out     TCCR0A, temp
        ldi     temp, (1<<WGM02) | (1<<CS00)
        out     TCCR0B, temp

        ldi     temp, 2                                 ; MUX = ADC2 (PB2)
        out     ADMUX, temp
        ldi     temp, (1<<ADC2D)                        ; disable PB2 digital input
        out     DIDR0, temp
        ldi     temp, (1<<ADEN) | (1<<ADPS1) | (1<<ADPS0)   ; ADC on, clk/8
        out     ADCSRA, temp

main:
        sbi     ADCSRA, ADSC                            ; start a conversion
wait:
        sbic    ADCSRA, ADSC                            ; ADSC clears when done
        rjmp    wait
        in      temp, ADCL
        out     OCR0AH, zero
        out     OCR0AL, temp
        rjmp    main
