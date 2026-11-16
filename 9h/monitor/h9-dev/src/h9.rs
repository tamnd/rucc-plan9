//! The numbers of the 9h hypercalls (plan 10.5). They are the same as in `9h/arm64/h9.h`. A call
//! is `HVC #0` with the number in X0 and the arguments in X1 to X5, and the result goes in X0. The
//! numbers are SMCCC fast calls in the range of the hypervisor vendor, so PSCI and 9h calls use
//! one path.

/// `H9version`: returns `REVISION`.
pub const VERSION: u32 = 0xc600_0000;
/// `H9consputc`: (c) writes the byte c to the console.
pub const CONS_PUTC: u32 = 0xc600_0001;
/// `H9consputs`: (pa, n) writes the n bytes at pa to the console, and returns n.
pub const CONS_PUTS: u32 = 0xc600_0002;
/// `H9consgetc`: returns the next input byte, or all ones when no byte waits.
pub const CONS_GETC: u32 = 0xc600_0003;
/// `H9sdinfo`: (unit) returns the number of 512-byte sectors of the disk unit, or -1 when the
/// unit does not exist.
pub const SD_INFO: u32 = 0xc600_0010;
/// `H9sdread`: (unit, lba, n, pa) reads n sectors from the sector lba to pa, and returns the number
/// it read. One call reads at most `SD_MAX` sectors.
pub const SD_READ: u32 = 0xc600_0011;
/// `H9sdwrite`: (unit, lba, n, pa) writes n sectors at pa to the sector lba, and returns the number
/// it wrote. One call writes at most `SD_MAX` sectors.
pub const SD_WRITE: u32 = 0xc600_0012;
/// `H9sdflush`: (unit) puts the writes of the unit on the host disk, and returns 0.
pub const SD_FLUSH: u32 = 0xc600_0013;
/// `H9fsreset`: () starts a new 9P session of `#Z/hostfs`, and returns 0.
pub const FS_RESET: u32 = 0xc600_0020;
/// `H9fswrite`: (pa, n) gives the n bytes at pa to the 9P server, and returns n.
pub const FS_WRITE: u32 = 0xc600_0021;
/// `H9fsread`: (pa, n) copies at most n bytes of the replies of the 9P server to pa, and returns
/// the number it copied, 0 when no reply waits.
pub const FS_READ: u32 = 0xc600_0022;

/// `H9snarfsize`: () reads the host clipboard into the buffer of the monitor, and returns its
/// length, at most `SNARF_MAX`.
pub const SNARF_SIZE: u32 = 0xc600_0030;
/// `H9snarfget`: (pa, n, off) copies at most n bytes of that buffer from off to pa, and returns the
/// number it copied.
pub const SNARF_GET: u32 = 0xc600_0031;
/// `H9snarfput`: (pa, n) makes the n bytes at pa the host clipboard, and returns n.
pub const SNARF_PUT: u32 = 0xc600_0032;
/// `H9ctl`: (pa, n) gives the text request of n bytes at pa to the monitor, and returns n, or -3
/// for a request that the monitor does not know.
pub const CTL: u32 = 0xc600_0040;

/// `H9screeninfo`: () returns the size of the screen, width | height<<16.
pub const SCREEN_INFO: u32 = 0xc600_0050;
/// `H9screenset`: (pa, width, height, chan) gives the framebuffer at pa, and returns 0, -2 when
/// it is not in the RAM, or -3 when the size or the channel is not that of the screen.
pub const SCREEN_SET: u32 = 0xc600_0051;
/// `H9screenflush`: (min, max) tells the monitor that the rectangle changed, with x | y<<16 for
/// each point, and returns 0.
pub const SCREEN_FLUSH: u32 = 0xc600_0052;
/// `H9mouseget`: () returns the next mouse event, x | y<<16 | buttons<<32, or -1 when no event
/// waits.
pub const MOUSE_GET: u32 = 0xc600_0060;
/// `H9kbdget`: () returns the next keyboard event, the rune | down<<32, with down 1 for a key down
/// and 0 for a key up, or -1 when no event waits.
pub const KBD_GET: u32 = 0xc600_0061;
/// `H9kbdopen`: (open) tells the monitor that the kernel opened `#b/kbd`, open 1, or closed it,
/// open 0, and returns 0.
pub const KBD_OPEN: u32 = 0xc600_0062;

/// `H9audiowrite`: (pa, n, speed, volume) gives the n bytes of samples at pa to the audio
/// output, at speed samples a second, with the volume left | right<<8, each 0 to 100. It returns
/// the number of bytes it took, which is less than n when the queue is full. One call gives at
/// most `AUDIO_MAX` bytes.
pub const AUDIO_WRITE: u32 = 0xc600_0070;
/// `H9audiobuffered`: () returns the number of bytes in the queue of the audio output that did
/// not play yet.
pub const AUDIO_BUFFERED: u32 = 0xc600_0071;

/// The largest snarf, 1 MiB.
pub const SNARF_MAX: u64 = 1 << 20;
/// The most bytes of one `H9audiowrite`, 64 KiB.
pub const AUDIO_MAX: u64 = 1 << 16;
/// The bytes of the queue of the audio output, 256 KiB, which is about 1.5 s at 44100 Hz.
pub const AUDIO_QUEUE: u64 = 1 << 18;
/// The largest request of `H9ctl`.
pub const CTL_MAX: u64 = 256;
/// The size of a sector of a disk unit.
pub const SD_SECTOR: u64 = 512;
/// The most sectors of one `H9sdread` or `H9sdwrite`, 64 KiB.
pub const SD_MAX: u64 = 128;

/// The revision of the interface, `H9revision`.
pub const REVISION: u64 = 1;
/// The result of a call that no device has: -1, as `NOT_SUPPORTED` of SMCCC.
pub const NOT_SUPPORTED: u64 = u64::MAX;
/// The result of a call with an address that is not in the RAM: -2.
pub const BAD_ADDRESS: u64 = u64::MAX - 1;
/// The result of a disk call that failed on the host, or that is outside the disk: -3.
pub const IO_ERROR: u64 = u64::MAX - 2;

/// The interrupt of the console input, `IRQh9cons`: SPI 16, which is INTID 48.
pub const CONS_SPI: u32 = 48;
/// The interrupt of the mouse, `IRQh9mouse`: SPI 17, which is INTID 49.
pub const MOUSE_SPI: u32 = 49;
/// The interrupt of the keyboard, `IRQh9kbd`: SPI 18, which is INTID 50.
pub const KBD_SPI: u32 = 50;
/// The interrupt of the console input on the amd64 machine: ISA IRQ 5, which no device of the
/// machine uses.
pub const CONS_IRQ: u32 = 5;
/// The interrupt of the mouse on the amd64 machine: ISA IRQ 10, which no device of the machine
/// uses.
pub const MOUSE_IRQ: u32 = 10;
/// The interrupt of the keyboard on the amd64 machine: ISA IRQ 11, which no device of the machine
/// uses.
pub const KBD_IRQ: u32 = 11;
