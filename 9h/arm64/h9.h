/*
 * The hypercalls of the 9h monitor (rucc-plan9, plan 10.5).
 * A call is HVC #0 with the number in R0 and the arguments
 * in R1 to R5, as hvccall in l.s does it. The result is in R0.
 * The numbers are SMCCC fast calls in the range of the
 * hypervisor vendor, so PSCI calls and 9h calls use one path.
 */
enum {
	H9version	= 0xC6000000,	/* () returns H9revision */
	H9consputc	= 0xC6000001,	/* (c) writes the byte c to the console */
	H9consputs	= 0xC6000002,	/* (pa, n) writes n bytes at pa, returns n */
	H9consgetc	= 0xC6000003,	/* () returns the next input byte, or -1 */

	H9revision	= 1,

	IRQh9cons	= SPI+16,	/* level, high while console input waits */
};

extern uvlong	h9call(uvlong, uvlong, uvlong);
