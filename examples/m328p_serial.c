/*
 * m328p_serial.c - "Hello" and echo over the ATmega328P USART.
 * Target: ATmega328P @ 1 MHz (factory fuses), 9600 baud 8N1 on PD1 (TXD) / PD0 (RXD).
 * Open View > Serial Monitor to read the output and type replies; every key
 * toggles the LED on PB5 (Arduino pin 13).
 */
#define F_CPU 1000000UL
#include <avr/io.h>

static void uart_init(void)
{
    UBRR0 = 12;                                 /* 1 MHz / (8 * 13) = 9615 baud with U2X */
    UCSR0A = (1 << U2X0);
    UCSR0B = (1 << RXEN0) | (1 << TXEN0);
    UCSR0C = (1 << UCSZ01) | (1 << UCSZ00);     /* 8 data bits, no parity, 1 stop bit */
}

static void uart_putc(char c)
{
    while (!(UCSR0A & (1 << UDRE0))) {
    }
    UDR0 = c;
}

static void uart_puts(const char *s)
{
    while (*s)
        uart_putc(*s++);
}

int main(void)
{
    DDRB |= (1 << DDB5);
    uart_init();
    uart_puts("Hello from the ATmega328P!\r\nType something: ");

    for (;;) {
        if (UCSR0A & (1 << RXC0)) {
            char c = UDR0;
            PINB = (1 << PINB5);                /* toggle the LED */
            uart_putc(c);
        }
    }
}
