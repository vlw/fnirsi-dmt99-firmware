/* Application image for the stock FNIRSI bootloader: it installs the update BIN at flash 0x6400
   (aliased at 0x08006400) and jumps there. 0x08040000 holds the stock settings page; stay below. */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00006400, LENGTH = 0x30000 - 0x6400
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 32K
}

ENTRY(_start)

SECTIONS
{
  .text : {
    KEEP(*(.text.header))
    *(.text .text.*)
  } > FLASH

  .rodata : ALIGN(4) {
    *(.srodata .srodata.* .rodata .rodata.*)
    . = ALIGN(4);
  } > FLASH

  .data : ALIGN(4) {
    _sdata = .;
    *(.sdata .sdata.* .data .data.*)
    . = ALIGN(4);
    _edata = .;
  } > RAM AT > FLASH
  _sidata = LOADADDR(.data);

  .bss (NOLOAD) : ALIGN(4) {
    _sbss = .;
    *(.sbss .sbss.* .bss .bss.*)
    . = ALIGN(4);
    _ebss = .;
  } > RAM

  PROVIDE(__global_pointer$ = _sdata + 0x800);
  _stack_top = ORIGIN(RAM) + LENGTH(RAM);

  /DISCARD/ : { *(.eh_frame .eh_frame_hdr) }
}

ASSERT(_start == 0x6400, "image must start at the application origin 0x6400");
ASSERT(_ebss + 4096 <= _stack_top, "less than 4 KiB left for the stack");
