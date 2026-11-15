/*
 * The console of the 9h machine (rucc-plan9, plan 10.5),
 * as a uart of port's devuart, so that kbdfs reads it as
 * /dev/eia0. Output is a hypercall. Input comes with ISA
 * interrupt 5, which stays high while bytes wait. The file
 * is that of 9h/arm64, but i8250console of pc64 main.c
 * makes it the console.
 * The structure is that of uartqemu.c.
 */
#include "u.h"
#include "../port/lib.h"
#include "../port/error.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "h9.h"

extern PhysUart h9physuart;

static Uart h9uart = {
	.name	= "uart0",
	.freq	= 1,
	.baud	= 115200,
	.phys	= &h9physuart,
};

static Uart*
pnp(void)
{
	return &h9uart;
}

static void
interrupt(Ureg*, void *arg)
{
	Uart *uart = arg;
	uvlong c;

	while((c = h9call(H9consgetc, 0, 0, 0, 0)) != ~0ULL)
		uartrecv(uart, c & 0xFF);
}

static void
enable(Uart *uart, int ie)
{
	if(ie)
		intrenable(IRQh9cons, interrupt, uart, BUSUNKNOWN, uart->name);
}

static void
disable(Uart*)
{
}

static void
kick(Uart *uart)
{
	int n;

	for(;;){
		if(uart->op >= uart->oe && uartstageoutput(uart) == 0)
			break;
		n = uart->oe - uart->op;
		h9call(H9consputs, PADDR(uart->op), n, 0, 0);
		uart->op += n;
	}
}

static void
dobreak(Uart*, int)
{
}

static int
baud(Uart *uart, int n)
{
	if(n <= 0)
		return -1;
	uart->baud = n;
	return 0;
}

static int
bits(Uart *uart, int n)
{
	if(n < 5 || n > 8)
		return -1;
	uart->bits = n;
	return 0;
}

static int
stop(Uart *uart, int n)
{
	if(n != 1 && n != 2)
		return -1;
	uart->stop = n;
	return 0;
}

static int
parity(Uart *uart, int n)
{
	if(n != 'n' && n != 'e' && n != 'o')
		return -1;
	uart->parity = n;
	return 0;
}

static void
donothing(Uart*, int)
{
}

static void
putc(Uart*, int c)
{
	h9call(H9consputc, c & 0xFF, 0, 0, 0);
}

static int
getc(Uart*)
{
	uvlong c;

	while((c = h9call(H9consgetc, 0, 0, 0, 0)) == ~0ULL)
		;
	return c & 0xFF;
}

void
i8250console(void)
{
	consuart = &h9uart;
	consuart->console = 1;
	uartctl(consuart, "l8 pn s1");
}

PhysUart h9physuart = {
	.name		= "h9",
	.pnp		= pnp,
	.enable		= enable,
	.disable	= disable,
	.kick		= kick,
	.dobreak	= dobreak,
	.baud		= baud,
	.bits		= bits,
	.stop		= stop,
	.parity		= parity,
	.modemctl	= donothing,
	.rts		= donothing,
	.dtr		= donothing,
	.fifo		= donothing,
	.getc		= getc,
	.putc		= putc,
};
