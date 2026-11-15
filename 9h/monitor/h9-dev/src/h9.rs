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
