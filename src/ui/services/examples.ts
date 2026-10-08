/** Bundled example programs (repo `examples/` folder) and new-file templates. */
const files = import.meta.glob('../../../examples/*.{asm,c}', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

export interface Example {
  name: string;
  title: string;
  description: string;
  text: string;
  /** Set for the documents used as "New file" templates. */
  template?: 'asm' | 'c';
}

const META: Record<string, { title: string; description: string }> = {
  'blink.asm': { title: 'Blink (assembly)', description: 'Toggle PB0 with a software delay loop' },
  'pwm_fade.asm': { title: 'PWM fade (assembly)', description: 'Fast PWM on OC0A, duty ramp in the overflow interrupt' },
  'button_interrupt.asm': { title: 'Button interrupt (assembly)', description: 'INT0 falling edge toggles an LED; idle sleep' },
  'adc_to_pwm.asm': { title: 'ADC to PWM (assembly)', description: 'Analog input on PB2 controls the PWM duty cycle' },
  'watchdog_sleep.asm': { title: 'Watchdog wake-up (assembly)', description: 'Power-down sleep, WDT interrupt every 0.5 s, CCP' },
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
];

export const SHOWCASE = EXAMPLES.filter((e) => !e.template);
