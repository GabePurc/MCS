/*
 * blink.c - toggles an LED on PB0 every 100 ms.
 * Target: ATtiny10 @ 1 MHz. Requires avr-gcc (Tools > Toolchain Options).
 */
#define F_CPU 1000000UL
#include <avr/io.h>
#include <util/delay.h>

int main(void)
{
    DDRB |= (1 << DDB0);            /* PB0 = output */

    for (;;) {
        PINB = (1 << PINB0);        /* writing 1 to PINx toggles the pin */
        _delay_ms(100);
    }
}
