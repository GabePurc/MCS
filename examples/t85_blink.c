/*
 * t85_blink.c - blinks an LED on PB3 (pin 2) with a Timer0 overflow interrupt.
 * Target: ATtiny85 @ 1 MHz (factory fuses). Needs avr-gcc.
 */
#define F_CPU 1000000UL
#include <avr/io.h>
#include <avr/interrupt.h>
#include <avr/sleep.h>

static volatile unsigned char ticks;

ISR(TIMER0_OVF_vect)
{
    /* 1 MHz / 1024 / 256 = 3.8 overflows per second */
    if (++ticks >= 2) {
        ticks = 0;
        PINB = (1 << PINB3);    /* toggle */
    }
}

int main(void)
{
    DDRB |= (1 << DDB3);
    TCCR0B = (1 << CS02) | (1 << CS00);     /* clk / 1024 */
    TIMSK |= (1 << TOIE0);
    set_sleep_mode(SLEEP_MODE_IDLE);
    sei();
    for (;;)
        sleep_mode();
}
