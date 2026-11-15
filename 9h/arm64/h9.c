/*
 * The 9h machine (rucc-plan9, plan 10.2): the hypercall,
 * and the PCI interrupt calls of gic.c for a machine with
 * no PCI bus.
 */
#include "u.h"
#include "../port/lib.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "ureg.h"
#include "h9.h"

uvlong
h9call(uvlong fn, uvlong a1, uvlong a2)
{
	Ureg u;

	memset(&u, 0, sizeof u);
	u.r0 = fn;
	u.r1 = a1;
	u.r2 = a2;
	hvccall(&u);
	return u.r0;
}

/*
 * gic.c calls these for an interrupt with a PCI tbdf.
 * The 9h machine has no PCI device, so no driver asks.
 */
void
pciintrenable(int, void (*)(Ureg*, void*), void*)
{
}

void
pciintrdisable(int, void (*)(Ureg*, void*), void*)
{
}
