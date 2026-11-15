/*
 * #Z, the host devices of the 9h machine (rucc-plan9, plan 10.5).
 *
 * hostfs is a 9P2000 channel to the file server of the
 * monitor, which serves a host directory. The boot script
 * mounts it on /mnt/term. The monitor answers each whole
 * message in the call that gives its last byte, so a read
 * only waits when no request is out; a write wakes it.
 *
 * snarf is the host clipboard. A read at offset 0 gets it
 * from the host again, because rio keeps its snarf file open
 * and seeks to 0 for each paste. The text written to a file
 * opened for writing goes to the host clipboard at close,
 * as with /dev/snarf of rio.
 *
 * ctl takes the requests quit and title TEXT.
 */
#include "u.h"
#include "../port/lib.h"
#include "../port/error.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "h9.h"

enum {
	Qdir,
	Qhostfs,
	Qsnarf,
	Qctl,
};

static Dirtab hostdir[] = {
	".",		{Qdir, 0, QTDIR},	0,	DMDIR|0555,
	"hostfs",	{Qhostfs},		0,	0660,
	"snarf",	{Qsnarf},		0,	0666,
	"ctl",		{Qctl},			0,	0220,
};

typedef struct Snarf Snarf;
struct Snarf {
	QLock;
	uchar	*buf;
	long	n;	/* bytes of text in buf */
	long	a;	/* bytes allocated */
};

static struct {
	Lock;
	int	open;
	QLock	rl;	/* one reader at a time */
	Rendez	r;
	ulong	wrote;	/* the writes, for the reader to wait on */
} fs;

static Chan*
hostattach(char *spec)
{
	return devattach('Z', spec);
}

static Walkqid*
hostwalk(Chan *c, Chan *nc, char **name, int nname)
{
	return devwalk(c, nc, name, nname, hostdir, nelem(hostdir), devgen);
}

static int
hoststat(Chan *c, uchar *db, int n)
{
	return devstat(c, db, n, hostdir, nelem(hostdir), devgen);
}

/* give the n bytes at a to the call fn through kernel memory */
static uvlong
h9give(ulong fn, void *a, long n)
{
	uchar *b;
	uvlong r;

	b = smalloc(n);
	if(waserror()){
		free(b);
		nexterror();
	}
	memmove(b, a, n);
	r = h9call(fn, PADDR(b), n, 0, 0);
	poperror();
	free(b);
	return r;
}

static void
snarfgrow(Snarf *s, long n)
{
	uchar *b;

	if(n <= s->a)
		return;
	if(n < 2*s->a)
		n = 2*s->a;
	if(n > H9snarfmax)
		n = H9snarfmax;
	b = realloc(s->buf, n);
	if(b == nil)
		error(Enomem);
	s->buf = b;
	s->a = n;
}

static void
snarfget(Snarf *s)
{
	uvlong n;

	s->n = 0;
	n = h9call(H9snarfsize, 0, 0, 0, 0);
	if(n == 0 || n > H9snarfmax)
		return;
	snarfgrow(s, n);
	n = h9call(H9snarfget, PADDR(s->buf), n, 0, 0);
	if(n <= s->a)
		s->n = n;
}

static Chan*
hostopen(Chan *c, int omode)
{
	if(c->qid.path == Qhostfs){
		lock(&fs);
		if(fs.open){
			unlock(&fs);
			error(Einuse);
		}
		fs.open = 1;
		unlock(&fs);
		if(h9call(H9fsreset, 0, 0, 0, 0) != 0){
			fs.open = 0;
			error("the monitor serves no host files");
		}
	}
	if(waserror()){
		if(c->qid.path == Qhostfs)
			fs.open = 0;
		nexterror();
	}
	c = devopen(c, omode, hostdir, nelem(hostdir), devgen);
	poperror();
	if(c->qid.path == Qsnarf)
		c->aux = smalloc(sizeof(Snarf));
	return c;
}

static void
hostclose(Chan *c)
{
	Snarf *s;

	if((c->flag & COPEN) == 0)
		return;
	switch((ulong)c->qid.path){
	case Qhostfs:
		lock(&fs);
		fs.open = 0;
		unlock(&fs);
		break;
	case Qsnarf:
		s = c->aux;
		if(s == nil)
			break;
		if(c->mode == OWRITE || c->mode == ORDWR)
			h9call(H9snarfput, s->n > 0 ? PADDR(s->buf) : 0, s->n, 0, 0);
		c->aux = nil;
		free(s->buf);
		free(s);
		break;
	}
}

static int
wrote(void *a)
{
	return fs.wrote != *(ulong*)a;
}

static long
hostread(Chan *c, void *a, long n, vlong off)
{
	uchar *b;
	uvlong r;
	ulong seen;
	Snarf *s;

	switch((ulong)c->qid.path){
	case Qdir:
		return devdirread(c, a, n, hostdir, nelem(hostdir), devgen);
	case Qhostfs:
		if(n > H9fsmax)
			n = H9fsmax;
		b = smalloc(n);
		qlock(&fs.rl);
		if(waserror()){
			qunlock(&fs.rl);
			free(b);
			nexterror();
		}
		for(;;){
			seen = fs.wrote;
			r = h9call(H9fsread, PADDR(b), n, 0, 0);
			if(r != 0)
				break;
			sleep(&fs.r, wrote, &seen);
		}
		if(r > n)
			error(Eio);
		memmove(a, b, r);
		poperror();
		qunlock(&fs.rl);
		free(b);
		return r;
	case Qsnarf:
		s = c->aux;
		qlock(s);
		if(waserror()){
			qunlock(s);
			nexterror();
		}
		if(off == 0)
			snarfget(s);
		if(off >= s->n)
			n = 0;
		else{
			if(n > s->n - off)
				n = s->n - off;
			memmove(a, s->buf + off, n);
		}
		poperror();
		qunlock(s);
		return n;
	}
	error(Egreg);
	return 0;
}

static long
hostwrite(Chan *c, void *a, long n, vlong off)
{
	Snarf *s;

	switch((ulong)c->qid.path){
	case Qhostfs:
		if(n > H9fsmax)
			error(Etoobig);
		if(h9give(H9fswrite, a, n) != n)
			error(Eio);
		lock(&fs);
		fs.wrote++;
		unlock(&fs);
		wakeup(&fs.r);
		return n;
	case Qsnarf:
		if(off < 0 || off + n > H9snarfmax)
			error(Etoobig);
		s = c->aux;
		qlock(s);
		if(waserror()){
			qunlock(s);
			nexterror();
		}
		snarfgrow(s, off + n);
		if(off > s->n)
			memset(s->buf + s->n, 0, off - s->n);
		memmove(s->buf + off, a, n);
		if(off + n > s->n)
			s->n = off + n;
		poperror();
		qunlock(s);
		return n;
	case Qctl:
		if(n > H9ctlmax)
			error(Etoobig);
		if(h9give(H9ctl, a, n) != n)
			error(Ebadctl);
		return n;
	}
	error(Eperm);
	return 0;
}

static void
hostinit(void)
{
	h9dumpinit();
}

Dev hostdevtab = {
	'Z',
	"host",

	devreset,
	hostinit,
	devshutdown,
	hostattach,
	hostwalk,
	hoststat,
	hostopen,
	devcreate,
	hostclose,
	hostread,
	devbread,
	hostwrite,
	devbwrite,
	devremove,
	devwstat,
};
