/*
 * The audio output of the 9h machine (rucc-plan9, plan 11.6).
 * It is a card of port's devaudio. A write gives the samples
 * to the monitor with H9audiowrite, and the monitor plays them
 * on the host. The samples wait in the queue of the monitor.
 * As in audiohda, a write returns when the queue has at most
 * delay samples, so the player of the host sets the pace.
 * The monitor applies the master volume. There is no input.
 * The file is the same for arm64 and amd64.
 */
#include "u.h"
#include "../port/lib.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "../port/error.h"
#include "io.h"
#include "../port/audioif.h"
#include "h9.h"

enum {
	Blocksize	= 4096,	/* bytes of one call */
	Bytespersample	= 4,	/* two channels of 16 bits */
	Wait		= 10,	/* ms between two looks at the queue */
};

typedef struct Ctlr Ctlr;
struct Ctlr {
	QLock;		/* the buffer, for writes on one fd from two procs */
	uchar	*buf;
	int	vol[2];	/* master, 0 to 100 */
};

enum {
	Vmaster,
	Vspeed,
	Vdelay,
	Nvol,
};

static Volume voltab[] = {
	[Vmaster] "master", 0, 100, Stereo, 0,
	[Vspeed] "speed", 0, 0, Absolute, 0,
	[Vdelay] "delay", 0, 0, Absolute, 0,
	0
};

static long
h9abuffered(Audio *)
{
	vlong n;

	n = h9call(H9audiobuffered, 0, 0, 0, 0);
	return n < 0 ? 0 : n;
}

static int
h9adone(void *)
{
	return 0;
}

static long
h9awrite(Audio *adev, void *vp, long n, vlong)
{
	Ctlr *ctlr;
	uchar *p, *e;
	long m;
	vlong r;

	ctlr = adev->ctlr;
	eqlock(ctlr);
	if(waserror()){
		qunlock(ctlr);
		nexterror();
	}
	p = vp;
	e = p + n;
	while(p < e){
		m = e - p;
		if(m > Blocksize)
			m = Blocksize;
		memmove(ctlr->buf, p, m);
		r = h9call(H9audiowrite, PADDR(ctlr->buf), m, adev->speed,
			ctlr->vol[0] | ctlr->vol[1]<<8);
		if(r < 0)
			error(Eio);
		p += r;
		if(r < m)
			tsleep(&up->sleep, h9adone, nil, Wait);
	}
	qunlock(ctlr);
	poperror();
	while(h9abuffered(adev) > adev->delay*Bytespersample)
		tsleep(&up->sleep, h9adone, nil, Wait);
	return n;
}

static int
h9agetvol(Audio *adev, int x, int a[2])
{
	Ctlr *ctlr;

	ctlr = adev->ctlr;
	switch(x){
	case Vmaster:
		a[0] = ctlr->vol[0];
		a[1] = ctlr->vol[1];
		break;
	case Vspeed:
		a[0] = adev->speed;
		break;
	case Vdelay:
		a[0] = adev->delay;
		break;
	}
	return 0;
}

static int
h9asetvol(Audio *adev, int x, int a[2])
{
	Ctlr *ctlr;

	ctlr = adev->ctlr;
	switch(x){
	case Vmaster:
		ctlr->vol[0] = a[0];
		ctlr->vol[1] = a[1];
		break;
	case Vspeed:
		if(a[0] < 1000 || a[0] > 192000)
			error(Ebadarg);
		adev->speed = a[0];
		break;
	case Vdelay:
		if(a[0] < Blocksize/Bytespersample)
			a[0] = Blocksize/Bytespersample;
		if(a[0] > H9audioqueue/Bytespersample/2)
			a[0] = H9audioqueue/Bytespersample/2;
		adev->delay = a[0];
		break;
	}
	return 0;
}

static long
h9avolread(Audio *adev, void *a, long n, vlong)
{
	return genaudiovolread(adev, a, n, 0, voltab, h9agetvol, 0);
}

static long
h9avolwrite(Audio *adev, void *a, long n, vlong)
{
	return genaudiovolwrite(adev, a, n, 0, voltab, h9asetvol, 0);
}

static long
h9astatus(Audio *adev, void *a, long n, vlong)
{
	return snprint(a, n, "bufsize %6d buffered %6ld\n", H9audioqueue, h9abuffered(adev));
}

static int
h9aprobe(Audio *adev)
{
	Ctlr *ctlr;

	if(adev->ctlrno > 0)
		return -1;
	if(h9call(H9audiobuffered, 0, 0, 0, 0) == ~0ULL)
		return -1;
	ctlr = mallocz(sizeof(Ctlr), 1);
	if(ctlr != nil)
		ctlr->buf = mallocalign(Blocksize, Blocksize, 0, 0);
	if(ctlr == nil || ctlr->buf == nil){
		free(ctlr);
		print("audioh9: no memory\n");
		return -1;
	}
	ctlr->vol[0] = ctlr->vol[1] = 100;
	adev->ctlr = ctlr;
	adev->speed = 44100;
	adev->delay = 1764;
	adev->write = h9awrite;
	adev->buffered = h9abuffered;
	adev->volread = h9avolread;
	adev->volwrite = h9avolwrite;
	adev->status = h9astatus;
	return 0;
}

void
audioh9link(void)
{
	addaudiocard("h9", h9aprobe);
}
