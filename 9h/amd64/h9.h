/*
 * The hypercalls of the 9h monitor (rucc-plan9, plan 10.5).
 * A call is OUTB to port H9port with the number in AX and
 * the arguments in BX, CX, SI and DI, as h9call in h9l.s does
 * it. The result is in AX.
 * The numbers are those of the arm64 machine, so the two
 * machines have one monitor.
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
	H9snarfsize	= 0xC6000030,	/* () reads the host clipboard, returns its length */
	H9snarfget	= 0xC6000031,	/* (pa, n, off) copies at most n bytes from off, returns the count */
	H9snarfput	= 0xC6000032,	/* (pa, n) sets the host clipboard, returns n */
	H9ctl		= 0xC6000040,	/* (pa, n) gives a text request, returns n */
	H9screeninfo	= 0xC6000050,	/* () returns width | height<<16, or -1 */
	H9screenset	= 0xC6000051,	/* (pa, width, height, chan) gives the framebuffer, returns 0 */
	H9screenflush	= 0xC6000052,	/* (min, max) the rectangle changed, x | y<<16 each, returns 0 */
	H9mouseget	= 0xC6000060,	/* () returns x | y<<16 | buttons<<32 of the next event, or -1 */
	H9kbdget	= 0xC6000061,	/* () returns rune | down<<32 of the next key event, or -1 */
	H9kbdopen	= 0xC6000062,	/* (open) #b/kbd is open, 1, or closed, 0, returns 0 */

	H9sdsecsize	= 512,		/* bytes in a sector */
	H9sdmax	= 128,		/* the most sectors of one call */
	H9fsmax	= 65536+24,	/* the largest 9P message of the host */
	H9snarfmax	= 1<<20,	/* the largest snarf */
	H9ctlmax	= 256,		/* the largest request of H9ctl */

	H9revision	= 1,

	IRQh9cons	= 5,		/* ISA IRQ 5, high while console input waits */
	IRQh9mouse	= 10,		/* ISA IRQ 10, high while a mouse event waits */
	IRQh9kbd	= 11,		/* ISA IRQ 11, high while a key event waits */
	H9port		= 0x509,	/* the port of the hypercall */
};

extern uvlong	h9call(uvlong, uvlong, uvlong, uvlong, uvlong);
