/*
 * The console of the 9h machine (rucc-plan9, plan 10.5),
 * as a uart of port's devuart, so that kbdfs reads it as
 * /dev/eia0. Output is a hypercall. Input comes with a
 * level interrupt that stays high while bytes wait.
 * The structure is that of uartqemu.c.
 */
#include "u.h"
#include "../port/lib.h"
#include "../port/error.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "ureg.h"
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

/*
 * The keys ^T^T p of old Plan 9: the console writes the state
 * of each process. When rio has the screen, the console
 * keyboard goes to a window, and a guest that hangs there has
 * no shell to run ps. Each byte, the ^T too, goes to the uart
 * as before, but the p that follows ^T^T does not. The
 * interrupt only wakes the kernel process h9dump, because the
 * stack of the interrupt is too small for the prints.
 */
static Rendez dumpr;
static int dumpwanted;

static int
wanted(void*)
{
	return dumpwanted;
}

static void
dumpproc(void*)
{
	int i;
	char *ps;
	Proc *p;
	Mach *m;
	Ureg *u;

	for(;;){
		sleep(&dumpr, wanted, nil);
		dumpwanted = 0;
		for(i = 0; i < conf.nproc; i++){
			p = proctab(i);	/* nil when the slot was never used */
			if(p == nil || p->state == Dead)
				continue;
			ps = p->psstate;
			if(ps == nil)
				ps = statename[p->state];
			m = p->mach;	/* it changes as the process runs */
			u = p->dbgreg;	/* the user registers, at a system call */
			iprint("%lud %s %s pc %#p qpc %#p nl %d mach %d upc %#p ulink %#p\n",
				p->pid, p->text, ps, p->pc, p->qpc, p->nlocks,
				m != nil ? m->machno : -1,
				u != nil ? u->pc : 0, u != nil ? u->link : 0);
		}
		for(i = 0; i < conf.nmach; i++)
			iprint("cpu%d ticks %lud\n", i, MACHP(i)->ticks);
		iprint("uart staged %ld queued %d\n",
			h9uart.iw >= h9uart.ir ? h9uart.iw - h9uart.ir : Stagesize - (h9uart.ir - h9uart.iw),
			h9uart.iq != nil ? qlen(h9uart.iq) : -1);
	}
}

/* devhost calls this at its init, from the first process */
void
h9dumpinit(void)
{
	kproc("h9dump", dumpproc, nil);
}

static int
debugkey(int c)
{
	static int ctrlt;

	if(c == 0x14){
		if(ctrlt < 2)
			ctrlt++;
		return 0;
	}
	if(ctrlt != 2){
		ctrlt = 0;
		return 0;
	}
	ctrlt = 0;
	if(c != 'p')
		return 0;
	dumpwanted = 1;
	wakeup(&dumpr);
	return 1;
}

static void
interrupt(Ureg*, void *arg)
{
	Uart *uart = arg;
	uvlong c;

	while((c = h9call(H9consgetc, 0, 0, 0, 0)) != ~0ULL)
		if(!debugkey(c & 0xFF))
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
uartconsinit(void)
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
