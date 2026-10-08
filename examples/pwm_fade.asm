; =============================================================================
;  pwm_fade.asm - 8-bit fast PWM on OC0A (PB0); the duty cycle ramps up in the
;  Timer0 overflow interrupt. Watch PB0 in the Waveform window.
;  Target: ATtiny10 @ 1 MHz
; =============================================================================
.include "tn10def.inc"

.def    temp    = r16
.def    duty    = r17
.def    zero    = r18

.cseg
.org 0x0000
        rjmp    reset
.org TIM0_OVFaddr
        rjmp    tim0_ovf

reset:
        clr     zero
        sbi     DDRB, DDB0                          ; OC0A pin is an output

        ldi     temp, (1<<COM0A1) | (1<<WGM00)      ; non-inverting, WGM0 = 0101
        out     TCCR0A, temp                        ; (fast PWM, 8-bit)
        ldi     temp, (1<<WGM02) | (1<<CS00)        ; clk/1
        out     TCCR0B, temp

        ldi     temp, (1<<TOIE0)                    ; overflow interrupt
        out     TIMSK0, temp
        clr     duty
        sei

main:
        rjmp    main

; -----------------------------------------------------------------------------
tim0_ovf:
        push    temp
        in      temp, SREG                          ; preserve flags
        push    temp

        inc     duty
        out     OCR0AH, zero                        ; 16-bit write: high byte first
        out     OCR0AL, duty

        pop     temp
        out     SREG, temp
        pop     temp
        reti
