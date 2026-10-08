/*
 * pwm_fade.c - fades an LED on PB0 (OC0A) up and down using 8-bit fast PWM
 * and the Timer0 overflow interrupt.
 * Target: ATtiny10 @ 1 MHz. Requires avr-gcc (Tools > Toolchain Options).
 */
#include <avr/io.h>
#include <avr/interrupt.h>

static volatile uint8_t duty;
static volatile int8_t direction = 1;

ISR(TIM0_OVF_vect)
{
    duty += direction;
    if (duty == 0xFF || duty == 0)
        direction = -direction;
    OCR0A = duty;
}

int main(void)
{
    DDRB |= (1 << DDB0);
    TCCR0A = (1 << COM0A1) | (1 << WGM00);  /* non-inverting fast PWM, 8-bit */
    TCCR0B = (1 << WGM02) | (1 << CS00);    /* clk/1 */
    TIMSK0 = (1 << TOIE0);
    sei();

    for (;;) {
        /* everything happens in the interrupt */
    }
}
