/*
 * #Z, the host devices of the 9h machine (rucc-plan9, plan 10.5).
 *
 * hostfs is a 9P2000 channel to the file server of the
 * monitor, which serves a host directory. The boot script
 * mounts it on /mnt/term. The monitor answers each whole
 * message in the call that gives its last byte, so a read
 * only waits when no request is out; a write wakes it.
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
};

static Dirtab hostdir[] = {
	".",		{Qdir, 0, QTDIR},	0,	DMDIR|0555,
	"hostfs",	{Qhostfs},		0,	0660,
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
	return devopen(c, omode, hostdir, nelem(hostdir), devgen);
}

static void
hostclose(Chan *c)
{
	if(c->qid.path == Qhostfs && (c->flag & COPEN) != 0){
		lock(&fs);
		fs.open = 0;
		unlock(&fs);
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
	}
	error(Egreg);
	return 0;
}

static long
hostwrite(Chan *c, void *a, long n, vlong)
{
	uchar *b;
	uvlong r;

	switch((ulong)c->qid.path){
	case Qhostfs:
		if(n > H9fsmax)
			error(Etoobig);
		b = smalloc(n);
		if(waserror()){
			free(b);
			nexterror();
		}
		memmove(b, a, n);
		r = h9call(H9fswrite, PADDR(b), n, 0, 0);
		if(r != n)
			error(Eio);
		poperror();
		free(b);
		lock(&fs);
		fs.wrote++;
		unlock(&fs);
		wakeup(&fs.r);
		return n;
	}
	error(Eperm);
	return 0;
}

Dev hostdevtab = {
	'Z',
	"host",

	devreset,
	devinit,
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
