/* ESP32-C3 test programs: code in the IROM window, read-only data in DROM (behind the code), data / bss in DRAM. */
ENTRY(_start)
SECTIONS {
  . = 0x42000000;
  .text : { KEEP(*(.text.start)) *(.text .text.*) }
  . = 0x3C010000;
  .rodata : { *(.rodata .rodata.*) *(.srodata .srodata.*) }
  . = 0x3FC80000;
  .data : { *(.data .data.*) *(.sdata .sdata.*) }
  .bss (NOLOAD) : { *(.bss .bss.*) *(.sbss .sbss.*) *(COMMON) }
  /DISCARD/ : { *(.eh_frame*) *(.riscv.attributes) *(.comment) }
}
