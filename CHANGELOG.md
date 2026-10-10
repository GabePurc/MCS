# Changelog

The release workflow copies the section for the tagged version into the GitHub release, and
the in-app updater shows it as the update notes.

## [Unreleased]

- **Custom microcontrollers** (Device > Custom Microcontroller...): choose flash, SRAM and
  EEPROM sizes, ports, timers, USARTs, SPI, I²C, ADC channels and more, from a few bytes up to
  8 MB of flash. The simulator generates the registers, interrupts and pins.
- **Instruction help**: hover an instruction in the editor to see what it does, how to use it
  and an example; the Instruction Set window shows the same and lists aliases like BRNE.
- **Chip View**: zoom into the memories to read every byte; click a memory to jump there.
- **Clear Output on Build / Run** option (Build menu).
- **Fixed**: Tab in the middle of a line moved the whole line.

## [0.2.0]

- **New microcontrollers**: ATmega328P (the Arduino Uno chip) with the ATmega48PA/88PA/168PA,
  and the ATtiny85 with the ATtiny25/45. USART, SPI, I²C, USI, EEPROM, 10-bit ADC, three timers,
  the ATtiny85's 64 MHz PLL timer, fuses (Arduino Uno preset), brown-out and boot loader reset.
- **Serial Monitor**: read what the program sends over the serial port (or any pin) and type
  replies.
- **Fixed**: building a document you had just edited used its previous text.
- **Chip View (3D)**: the inside of the chip with a live die: executed code heat map and PC,
  SRAM and stack, registers, the instruction being decoded, SREG, peripheral activity and pin
  levels on the bond wires. Shadows and ambient occlusion; a flat die view too.
- **Speed control**: run at a fixed CPU clock from 1 Hz upward, at real time (or a multiple),
  or as fast as possible (Speed > Custom...).
- **Windows**: every tool window can float or open in its own window (right-click its tab).
- **Device Info**: specifications, speed grades, memory map, pins, vectors and the die.
- **Instruction Set** window stays open while you code; descriptions, encodings and an
  assembly <-> machine code converter.
- **Machine code files** (.mc): write programs as raw instruction words with live disassembly.
- **Pins**: signal generators (square waves, pulse bursts) and push buttons.
- **Fixed**: changing the external clock frequency now changes the MCU clock; Supply & Clock
  can select the clock source and prescaler.
- **Updates**: Help > Check for Updates installs new versions from inside the app.

## [0.1.0]

- First release: ATtiny4/5/9/10 simulator, assembler, avr-gcc integration, debugger.
