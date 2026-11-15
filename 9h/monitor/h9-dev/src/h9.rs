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

/// The revision of the interface, `H9revision`.
pub const REVISION: u64 = 1;
/// The result of a call that no device has: -1, as `NOT_SUPPORTED` of SMCCC.
pub const NOT_SUPPORTED: u64 = u64::MAX;
/// The result of a call with an address that is not in the RAM: -2.
pub const BAD_ADDRESS: u64 = u64::MAX - 1;

/// The interrupt of the console input, `IRQh9cons`: SPI 16, which is INTID 48.
pub const CONS_SPI: u32 = 48;
