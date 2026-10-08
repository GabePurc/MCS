/** Bundled example programs (repo `examples/` folder) and new-file templates. */
const files = import.meta.glob('../../../examples/*.{asm,c,mc}', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

export interface Example {
  name: string;
  title: string;
  description: string;
  text: string;
  /** Set for the documents used as "New file" templates. */
  template?: 'asm' | 'c' | 'mc';
}

const META: Record<string, { title: string; description: string }> = {
  'blink.asm': { title: 'Blink (assembly)', description: 'Toggle PB0 with a software delay loop' },
  'pwm_fade.asm': { title: 'PWM fade (assembly)', description: 'Fast PWM on OC0A, duty ramp in the overflow interrupt' },
  'button_interrupt.asm': { title: 'Button interrupt (assembly)', description: 'INT0 falling edge toggles an LED; idle sleep' },
  'adc_to_pwm.asm': { title: 'ADC to PWM (assembly)', description: 'Analog input on PB2 controls the PWM duty cycle' },
  'watchdog_sleep.asm': { title: 'Watchdog wake-up (assembly)', description: 'Power-down sleep, WDT interrupt every 0.5 s, CCP' },
  'blink.mc': { title: 'Blink (machine code)', description: 'The blink program written as raw instruction words' },
  'blink.c': { title: 'Blink (C)', description: '_delay_ms toggle loop (needs avr-gcc)' },
  'pwm_fade.c': { title: 'PWM fade (C)', description: 'Timer interrupt fading an LED (needs avr-gcc)' },
};

const ASM_TEMPLATE = `; ATtiny10 assembly program
.include "tn10def.inc"

.def    temp    = r16

.cseg
.org 0x0000
        rjmp    reset

reset:
        sbi     DDRB, DDB0          ; PB0 = output

main:
        rjmp    main
`;

const MC_TEMPLATE = `; Machine code for the ATtiny10 - write instruction words directly.
;   E00F               one 16-bit word in hex (0x and $ prefixes are optional)
;   0b1110 0000 0000 1111   a word in binary (0b, then digit groups up to 16 bits)
;   940C0010           8 hex digits = a two-word instruction (not on the ATtiny10)
;   @0x0010            continue at a byte address
; The disassembly appears at the end of each line. Encodings: Help > Instruction Set (F1).

@0x0000
E001        ; ldi r16, 0x01    -> 1110 KKKK dddd KKKK
B901        ; out DDRB, r16    -> PB0 = output
B900        ; out PINB, r16    -> toggle PB0 (loop start)
CFFE        ; rjmp .-4         -> jump back to the toggle
`;

const C_TEMPLATE = `#include <avr/io.h>

int main(void)
{
    DDRB |= (1 << DDB0);

    for (;;) {
    }
}
`;

export const EXAMPLES: Example[] = [
  ...Object.entries(files)
    .map(([path, text]) => {
      const name = path.slice(path.lastIndexOf('/') + 1);
      const meta = META[name] ?? { title: name, description: '' };
      return { name, text, ...meta };
    })
    .sort((a, b) => Object.keys(META).indexOf(a.name) - Object.keys(META).indexOf(b.name)),
  { name: 'template.asm', title: 'Assembly template', description: '', text: ASM_TEMPLATE, template: 'asm' as const },
  { name: 'template.c', title: 'C template', description: '', text: C_TEMPLATE, template: 'c' as const },
  { name: 'template.mc', title: 'Machine code template', description: '', text: MC_TEMPLATE, template: 'mc' as const },
];

export const SHOWCASE = EXAMPLES.filter((e) => !e.template);
