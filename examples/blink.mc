; =============================================================================
;  blink.mc - blink.asm written directly as machine code (ATtiny10)
;  Toggles PB0 roughly every 50 ms at 1 MHz.
;
;  Each line holds one instruction word in hex (or binary with 0b). The
;  disassembly of every line appears at its end while you type. Look up the
;  encodings in Help > Instruction Set (F1); its converter turns assembly into
;  words and back. Build with F7 and step with F10/F11 like any source file.
; =============================================================================

@0x0000
C000        ; rjmp reset          1100 kkkk kkkk kkkk   k = 0 (next word)

; reset:
E000        ; ldi r16, 0x00       1110 KKKK dddd KKKK   d = 16 -> dddd = 0000
BF0E        ; out SPH, r16        1011 1AAr rrrr AAAA   A = 0x3E
E50F        ; ldi r16, 0x5F       RAMEND = 0x5F
BF0D        ; out SPL, r16        A = 0x3D
0b1001_1010 0000_1000             ; sbi DDRB, 0 (1001 1010 AAAA Abbb: A = 1, b = 0)

; loop:
9A00        ; sbi PINB, 0         writing 1 to PINx toggles the pin
D001        ; rcall delay         1101 kkkk kkkk kkkk   k = +1
CFFD        ; rjmp loop           k = -3

; delay: about 50 000 cycles
E421        ; ldi r18, 65
EF1F        ; ldi r17, 255        (outer)
951A        ; dec r17             (inner)
F7F1        ; brne inner          1111 01kk kkkk k001   k = -2
952A        ; dec r18
F7D9        ; brne outer          k = -5
9508        ; ret
