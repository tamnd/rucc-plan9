/*
 * The screen of the 9h machine (rucc-plan9, plan 10.2 and 10.5),
 * from bcm/screen.c of the tree. The framebuffer is in the RAM of
 * the kernel. H9screeninfo gives the size that the monitor has,
 * H9screenset gives the monitor the address of the framebuffer,
 * and H9screenflush tells the monitor which rectangle changed.
 * The cursor is the software cursor of port/swcursor.c.
 * This is the file of arm64 with the screen.h of pc and with
 * vgascreen, which ishwimage of that screen.h uses.
 */

#include "u.h"
#include "../port/lib.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"
#include "h9.h"

#define	Image	IMAGE
#include <draw.h>
#include <memdraw.h>
#include <cursor.h>
/* not "screen.h": mk copies it from ../pc only for the files that need it */
#include "../pc/screen.h"

enum {
	Tabstop		= 4,
	Scroll		= 8,
	Depth		= 32,
};

Memimage *gscreen;

static Memdata xgdata;

static Memimage xgscreen =
{
	{ 0, 0, 0, 0 },	/* r */
	{ 0, 0, 0, 0 },	/* clipr */
	Depth,			/* depth */
	4,			/* nchan */
	XRGB32,			/* chan */
	nil,			/* cmap */
	&xgdata,		/* data */
	0,			/* zero */
	0, 			/* width in words of a single scan line */
	0,			/* layer */
	0,			/* flags */
};

static Memimage *conscol;
static Memimage *back;
static Memsubfont *memdefont;

/* ishwimage of pc/screen.h looks at gscreendata of the first screen */
VGAscr vgascreen[1];

static Lock screenlock;

static Point	curpos;
static int	h, w;
static Rectangle window;

static void myscreenputs(char *s, int n);
static void screenputc(char *buf);
static void screenwin(void);

/* called from devmouse */
enum
{
	CMaccelerated,
	CMlinear,
	CMres,
};

static Cmdtab mousectlmsg[] =
{
	CMaccelerated,		"accelerated",		0,
	CMlinear,		"linear",		1,
	CMres,			"res",			0,
};

void
mousectl(Cmdbuf *cb)
{
	Cmdtab *ct;

	ct = lookupcmd(cb, mousectlmsg, nelem(mousectlmsg));
	switch(ct->index){
	case CMaccelerated:
		mouseaccelerate(cb->nf == 1? 1: atoi(cb->f[1]));
		break;
	case CMlinear:
		mouseaccelerate(0);
		break;
	case CMres:
		/* lib/profile sets the resolution of a PS/2 mouse; the 9h mouse has none */
		break;
	}
}

void
cursoron(void)
{
	swcursorhide(0);
	swcursordraw(mousexy());
}

void
cursoroff(void)
{
	swcursorhide(0);
}

void
setcursor(Cursor* curs)
{
	swcursorload(curs);
}

int
hwdraw(Memdrawparam *par)
{
	Memimage *dst, *src, *mask;

	if((dst = par->dst) == nil || dst->data == nil)
		return 0;
	if((src = par->src) && src->data == nil)
		src = nil;
	if((mask = par->mask) && mask->data == nil)
		mask = nil;

	if(dst->data->bdata == xgdata.bdata)
		swcursoravoid(par->r);
	if(src && src->data->bdata == xgdata.bdata)
		swcursoravoid(par->sr);
	if(mask && mask->data->bdata == xgdata.bdata)
		swcursoravoid(par->mr);

	return 0;
}

/*
 * The size of the screen: vgasize=WxHxD of the plan9.ini,
 * or else the size that the monitor gives. The depth is
 * always 32.
 */
static int
screensize(void)
{
	char *p;
	char *f[3];
	int width, height;
	uvlong info;

	p = getconf("vgasize");
	if(p != nil && getfields(p, f, nelem(f), 0, "x") >= 2 &&
	    (width = atoi(f[0])) >= 16 && (height = atoi(f[1])) > 0){
		xgscreen.r.max = Pt(width, height);
		return 0;
	}
	info = h9call(H9screeninfo, 0, 0, 0, 0);
	if(info == (uvlong)-1)
		return -1;
	width = info & 0xffff;
	height = (info >> 16) & 0xffff;
	if(width < 16 || height <= 0)
		return -1;
	xgscreen.r.max = Pt(width, height);
	return 0;
}

void
screeninit(void)
{
	uchar *fb;
	ulong n;

	if(screensize() < 0)
		return;
	xgscreen.clipr = xgscreen.r;
	gscreen = &xgscreen;
	gscreen->width = wordsperline(gscreen->r, gscreen->depth);
	n = gscreen->width * sizeof(ulong) * Dy(gscreen->r);
	fb = xspanalloc(n, BY2PG, 0);
	if(fb == nil){
		print("h9screen: no memory for a %dx%d framebuffer\n",
			Dx(gscreen->r), Dy(gscreen->r));
		gscreen = nil;
		return;
	}
	if(h9call(H9screenset, PADDR(fb), Dx(gscreen->r), Dy(gscreen->r), XRGB32) != 0){
		print("h9screen: the monitor refuses the framebuffer\n");
		gscreen = nil;
		return;
	}
	memsetchan(&xgscreen, XRGB32);
	conf.monitor = 1;
	xgdata.bdata = fb;
	xgdata.ref = 1;
	vgascreen[0].gscreen = gscreen;
	vgascreen[0].gscreendata = &xgdata;
	vgascreen[0].softscreen = 1;

	memimageinit();
	memdefont = getmemdefont();
	screenwin();
	myscreenputs(kmesg.buf, kmesg.n);
	screenputs = myscreenputs;
	swcursorinit();
}

/*
 * The link function: links() of main.c calls it before
 * chandevreset, so conf.monitor is set when devmouse starts.
 */
void
h9screenlink(void)
{
	screeninit();
}

void
flushmemscreen(Rectangle r)
{
	if(gscreen == nil || !rectclip(&r, gscreen->r))
		return;
	h9call(H9screenflush, r.min.x | r.min.y<<16, r.max.x | r.max.y<<16, 0, 0);
}

Memdata*
attachscreen(Rectangle *r, ulong *chan, int* d, int *width, int *softscreen)
{
	if(gscreen == nil)
		return nil;

	*r = gscreen->r;
	*d = gscreen->depth;
	*chan = gscreen->chan;
	*width = gscreen->width;
	*softscreen = 1;

	gscreen->data->ref++;
	return gscreen->data;
}

void
getcolor(ulong p, ulong *pr, ulong *pg, ulong *pb)
{
	USED(p, pr, pg, pb);
}

int
setcolor(ulong p, ulong r, ulong g, ulong b)
{
	USED(p, r, g, b);
	return 0;
}

void
blankscreen(int)
{
}

static void
myscreenputs(char *s, int n)
{
	int i;
	Rune r;
	char buf[4];

	if(!islo()) {
		/* don't deadlock trying to print in interrupt */
		if(!canlock(&screenlock))
			return;	
	}
	else {
		while(!canlock(&screenlock))
			;
	}

	while(n > 0){
		i = chartorune(&r, s);
		if(i == 0){
			s++;
			--n;
			continue;
		}
		memmove(buf, s, i);
		buf[i] = 0;
		n -= i;
		s += i;
		screenputc(buf);
	}
	unlock(&screenlock);
}

static void
screenwin(void)
{
	char *greet;
	Memimage *orange;
	Point p;
	Rectangle r;

	back = memwhite;
	conscol = memblack;

	orange = allocmemimage(Rect(0, 0, 1, 1), RGB16);
	orange->flags |= Frepl;
	orange->clipr = gscreen->r;
	orange->data->bdata[0] = 0x40;		/* magic: colour? */
	orange->data->bdata[1] = 0xfd;		/* magic: colour? */

	w = memdefont->info[' '].width;
	h = memdefont->height;

	r = insetrect(gscreen->r, 4);

	memimagedraw(gscreen, r, memblack, ZP, memopaque, ZP, S);
	window = insetrect(r, 4);
	memimagedraw(gscreen, window, memwhite, ZP, memopaque, ZP, S);

	memimagedraw(gscreen, Rect(window.min.x, window.min.y,
		window.max.x, window.min.y + h + 5 + 6), orange, ZP, nil, ZP, S);
	freememimage(orange);
	window = insetrect(window, 5);

	greet = " Plan 9 Console ";
	p = addpt(window.min, Pt(10, 0));
	memimagestring(gscreen, p, conscol, ZP, memdefont, greet);
	flushmemscreen(r);
	window.min.y += h + 6;
	curpos = window.min;
	window.max.y = window.min.y + ((window.max.y - window.min.y) / h) * h;
}

static void
scroll(void)
{
	int o;
	Point p;
	Rectangle r;

	o = Scroll*h;
	r = Rpt(window.min, Pt(window.max.x, window.max.y-o));
	p = Pt(window.min.x, window.min.y+o);
	memimagedraw(gscreen, r, gscreen, p, nil, p, S);
	flushmemscreen(r);
	r = Rpt(Pt(window.min.x, window.max.y-o), window.max);
	memimagedraw(gscreen, r, back, ZP, nil, ZP, S);
	flushmemscreen(r);

	curpos.y -= o;
}

static void
screenputc(char *buf)
{
	int w;
	uint pos;
	Point p;
	Rectangle r;
	static int *xp;
	static int xbuf[256];

	if (xp < xbuf || xp >= &xbuf[nelem(xbuf)])
		xp = xbuf;

	switch (buf[0]) {
	case '\n':
		if (curpos.y + h >= window.max.y)
			scroll();
		curpos.y += h;
		screenputc("\r");
		break;
	case '\r':
		xp = xbuf;
		curpos.x = window.min.x;
		break;
	case '\t':
		p = memsubfontwidth(memdefont, " ");
		w = p.x;
		if (curpos.x >= window.max.x - Tabstop * w)
			screenputc("\n");

		pos = (curpos.x - window.min.x) / w;
		pos = Tabstop - pos % Tabstop;
		*xp++ = curpos.x;
		r = Rect(curpos.x, curpos.y, curpos.x + pos * w, curpos.y + h);
		memimagedraw(gscreen, r, back, back->r.min, nil, back->r.min, S);
		flushmemscreen(r);
		curpos.x += pos * w;
		break;
	case '\b':
		if (xp <= xbuf)
			break;
		xp--;
		r = Rect(*xp, curpos.y, curpos.x, curpos.y + h);
		memimagedraw(gscreen, r, back, back->r.min, nil, back->r.min, S);
		flushmemscreen(r);
		curpos.x = *xp;
		break;
	case '\0':
		break;
	default:
		p = memsubfontwidth(memdefont, buf);
		w = p.x;

		if (curpos.x >= window.max.x - w)
			screenputc("\n");

		*xp++ = curpos.x;
		r = Rect(curpos.x, curpos.y, curpos.x + w, curpos.y + h);
		memimagedraw(gscreen, r, back, back->r.min, nil, back->r.min, S);
		memimagestring(gscreen, curpos, conscol, ZP, memdefont, buf);
		flushmemscreen(r);
		curpos.x += w;
		break;
	}
}
