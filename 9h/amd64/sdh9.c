/*
 * The disks of the 9h machine (rucc-plan9, plan 10.5),
 * for port's devsd. Each unit is a host file of the
 * monitor. A read or a write is one hypercall of at most
 * H9sdmax sectors, which the monitor does before it
 * returns, so the driver has no interrupt. The structure
 * is that of port/sdram.c.
 */
#include "u.h"
#include "../port/lib.h"
#include "../port/error.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "h9.h"

#include "../port/sd.h"

enum {
	Nunit	= 4,	/* the same as MAX_UNITS of the monitor */
};

extern SDifc sdh9ifc;

static SDev*
h9pnp(void)
{
	SDev *sdev;

	if(h9call(H9version, 0, 0, 0, 0) != H9revision)
		return nil;
	sdev = malloc(sizeof(SDev));
	if(sdev == nil)
		return nil;
	sdev->idno = 'H';
	sdev->ifc = &sdh9ifc;
	sdev->nunit = Nunit;
	return sdev;
}

static int
h9verify(SDunit *unit)
{
	if(h9call(H9sdinfo, unit->subno, 0, 0, 0) == ~0ULL)
		return 0;
	unit->inquiry[0] = 0;
	unit->inquiry[1] = 0;
	unit->inquiry[4] = sizeof unit->inquiry - 4;
	snprint((char*)unit->inquiry+8, sizeof unit->inquiry - 8, "9h disk %d", unit->subno);
	return 1;
}

static int
h9online(SDunit *unit)
{
	uvlong n;

	if(unit->sectors != 0)
		return 1;
	n = h9call(H9sdinfo, unit->subno, 0, 0, 0);
	if(n == ~0ULL || n == 0)
		return 0;
	unit->sectors = n;
	unit->secsize = H9sdsecsize;
	return 2;
}

static char*
h9rctl(SDunit *unit, char *p, char *e)
{
	return seprint(p, e, "geometry %llud %lud\n", unit->sectors, unit->secsize);
}

static long
h9bio(SDunit *unit, int, int write, void *data, long nb, uvlong bno)
{
	uchar *a;
	long done, n;
	uvlong r;

	a = data;
	for(done = 0; done < nb; done += r){
		n = nb - done;
		if(n > H9sdmax)
			n = H9sdmax;
		r = h9call(write ? H9sdwrite : H9sdread, unit->subno, bno + done, n, PADDR(a + done*H9sdsecsize));
		if(r == 0 || r > n)
			error(Eio);
	}
	return nb * unit->secsize;
}

static int
h9rio(SDreq *r)
{
	int i, rw, count;
	uvlong lba;

	if((i = sdfakescsi(r)) != SDnostatus)
		return r->status = i;
	if((i = sdfakescsirw(r, &lba, &count, &rw)) != SDnostatus)
		return i;
	r->rlen = h9bio(r->unit, r->lun, rw == SDwrite, r->data, count, lba);
	return r->status = SDok;
}

SDifc sdh9ifc = {
	.name	= "h9",
	.pnp	= h9pnp,
	.verify	= h9verify,
	.online	= h9online,
	.rctl	= h9rctl,
	.bio	= h9bio,
	.rio	= h9rio,
};
