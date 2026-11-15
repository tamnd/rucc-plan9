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
	H9sdinfo	= 0xC6000010,	/* (unit) returns the number of sectors, or -1 */
	H9sdread	= 0xC6000011,	/* (unit, lba, n, pa) returns the sectors read */
	H9sdwrite	= 0xC6000012,	/* (unit, lba, n, pa) returns the sectors written */
	H9sdflush	= 0xC6000013,	/* (unit) returns 0 */
	H9fsreset	= 0xC6000020,	/* () starts a new 9P session of #Z/hostfs */
	H9fswrite	= 0xC6000021,	/* (pa, n) gives n bytes of 9P to the host, returns n */
	H9fsread	= 0xC6000022,	/* (pa, n) gets at most n bytes of 9P replies, returns the count */

	H9sdsecsize	= 512,		/* bytes in a sector */
	H9sdmax	= 128,		/* the most sectors of one call */
	H9fsmax	= 65536+24,	/* the largest 9P message of the host */

	H9revision	= 1,

	IRQh9cons	= SPI+16,	/* level, high while console input waits */
};

extern uvlong	h9call(uvlong, uvlong, uvlong, uvlong, uvlong);
