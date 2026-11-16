/*
 * The keyboard of the 9h machine (rucc-plan9). #b/kbd gives
 * the messages that kbdfs reads in place of /dev/scancode:
 * r and a rune for a key down, R and a rune for a key up,
 * each with a NUL at the end. The monitor keeps the events of
 * the host window and of the session script. Its interrupt
 * stays high while an event waits. The open and the last
 * close tell the monitor, which then sends typed text to the
 * keyboard and not to the console.
 * The file is the same for arm64 and amd64.
 */
#include "u.h"
#include "../port/lib.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "../port/error.h"
#include "io.h"
#include "h9.h"

enum {
	Qdir,
	Qkbd,
};

static Dirtab kbdtab[] = {
	".",	{Qdir, 0, QTDIR},	0,	0555,
	"kbd",	{Qkbd, 0},		0,	0440,
};

static Queue *kbdq;
static Ref kbdref;
static int kbdon;

static void
interrupt(Ureg*, void*)
{
	char buf[1+UTFmax+1];
	uvlong e;
	Rune r;
	int n;

	while((e = h9call(H9kbdget, 0, 0, 0, 0)) != (uvlong)-1){
		r = e & 0x1FFFFF;
		if(r == 0)
			continue;
		buf[0] = (e>>32) & 1 ? 'r' : 'R';
		n = runetochar(buf+1, &r);
		buf[1+n] = 0;
		qiwrite(kbdq, buf, n+2);
	}
}

static void
kbdinit(void)
{
	if(!conf.monitor)
		return;
	kbdq = qopen(4*1024, Qmsg, nil, nil);
	if(kbdq == nil)
		panic("h9kbd: no queue");
	intrenable(IRQh9kbd, interrupt, nil, BUSUNKNOWN, "h9kbd");
	kbdon = 1;
}

static Chan*
kbdattach(char *spec)
{
	if(!kbdon)
		error(Enodev);
	return devattach(L'b', spec);
}

static Walkqid*
kbdwalk(Chan *c, Chan *nc, char **name, int nname)
{
	return devwalk(c, nc, name, nname, kbdtab, nelem(kbdtab), devgen);
}

static int
kbdstat(Chan *c, uchar *dp, int n)
{
	return devstat(c, dp, n, kbdtab, nelem(kbdtab), devgen);
}

static Chan*
kbdopen(Chan *c, int omode)
{
	if(c->qid.path != Qkbd)
		return devopen(c, omode, kbdtab, nelem(kbdtab), devgen);
	if(!iseve())
		error(Eperm);
	if(waserror()){
		decref(&kbdref);
		nexterror();
	}
	if(incref(&kbdref) != 1)
		error(Einuse);
	/* a monitor that does not know the call gives -1 */
	if(h9call(H9kbdopen, 1, 0, 0, 0) != 0)
		error(Enodev);
	c = devopen(c, omode, kbdtab, nelem(kbdtab), devgen);
	poperror();
	return c;
}

static void
kbdclose(Chan *c)
{
	if((c->flag & COPEN) == 0 || c->qid.path != Qkbd)
		return;
	if(decref(&kbdref) == 0){
		h9call(H9kbdopen, 0, 0, 0, 0);
		qflush(kbdq);
	}
}

static long
kbdread(Chan *c, void *a, long n, vlong)
{
	if(c->qid.path == Qdir)
		return devdirread(c, a, n, kbdtab, nelem(kbdtab), devgen);
	if(c->qid.path == Qkbd)
		return qread(kbdq, a, n);
	error(Egreg);
	return 0;
}

static long
kbdwrite(Chan*, void*, long, vlong)
{
	error(Eperm);
	return 0;
}

Dev h9kbddevtab = {
	L'b',
	"h9kbd",

	devreset,
	kbdinit,
	devshutdown,
	kbdattach,
	kbdwalk,
	kbdstat,
	kbdopen,
	devcreate,
	kbdclose,
	kbdread,
	devbread,
	kbdwrite,
	devbwrite,
	devremove,
	devwstat,
};
