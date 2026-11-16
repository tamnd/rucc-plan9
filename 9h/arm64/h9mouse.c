/*
 * The mouse of the 9h machine (rucc-plan9, plan 10.5). The
 * monitor keeps the mouse events of the host window and of
 * the session script. Its interrupt is a level interrupt that
 * stays high while an event waits. Each event is absolute, in
 * the pixels of the screen, so it goes to absmousetrack of
 * port's devmouse.
 */
#include "u.h"
#include "../port/lib.h"
#include "../port/error.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "h9.h"

#define	Image	IMAGE
#include <draw.h>
#include <memdraw.h>
#include <cursor.h>
#include "screen.h"

static void
interrupt(Ureg*, void*)
{
	uvlong e;

	while((e = h9call(H9mouseget, 0, 0, 0, 0)) != (uvlong)-1)
		absmousetrack(e & 0xFFFF, (e >> 16) & 0xFFFF, (e >> 32) & 0x1F, TK2MS(MACHP(0)->ticks));
}

void
h9mouselink(void)
{
	if(!conf.monitor)
		return;
	intrenable(IRQh9mouse, interrupt, nil, BUSUNKNOWN, "h9mouse");
}
