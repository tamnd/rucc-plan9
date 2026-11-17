/*
 * The probe of r9 hostcheck (rucc-plan9, plan 11.9). It is a test
 * rig, not a program of the claim. It runs in a window of rio and
 * reads /dev/kbd and /dev/mouse of the window. For each message it
 * writes a line to the kernel console, where r9 reads it in the
 * console log of the monitor:
 *	R9-PROBE ready		the two files are open
 *	R9-PROBE kbd T R...	a message of /dev/kbd: its type k, K
 *				or c, and its runes in hex
 *	R9-PROBE mouse X Y B	a change of the buttons, at X, Y
 *	R9-PROBE snarf HEX	the bytes of /dev/snarf, at Ctrl-V
 *	R9-PROBE snarfed	after Ctrl-C wrote r9-probe-snarf
 *				to /dev/snarf
 *	R9-PROBE audio N	after it wrote N bytes of the
 *				pattern of the audio check to
 *				/dev/audio, when all keys are up
 *				after Ctrl-D
 *	R9-PROBE end		after the audio, before the probe
 *				stops
 *	R9-PROBE error TEXT	a file did not open
 */
#include <u.h>
#include <libc.h>

enum {
	Mousemsg	= 1+4*12,	/* m, then 4 numbers of 12 bytes */
	Snarfmax	= 1024,
	Audioframes	= 8192,	/* 16-bit stereo frames of the pattern */
};

static int cons;

static void
out(char *fmt, ...)
{
	char buf[1024], *p, *e;
	va_list arg;

	e = buf + sizeof buf - 1;
	p = seprint(buf, e, "R9-PROBE ");
	va_start(arg, fmt);
	p = vseprint(p, e, fmt, arg);
	va_end(arg);
	*p++ = '\n';
	write(cons, buf, p - buf);
}

static void
mouseproc(int fd)
{
	char buf[Mousemsg+1], *f[5];
	int b, last;

	last = 0;
	while(readn(fd, buf, Mousemsg) == Mousemsg){
		buf[Mousemsg] = 0;
		if(buf[0] != 'm' || tokenize(buf+1, f, nelem(f)) < 3)
			continue;
		b = atoi(f[2]);
		if(b != last){
			out("mouse %d %d %d", atoi(f[0]), atoi(f[1]), b);
			last = b;
		}
	}
	exits(nil);
}

static void
paste(void)
{
	char buf[Snarfmax], hex[2*Snarfmax+1], *p;
	int fd, i, n;

	fd = open("/dev/snarf", OREAD);
	if(fd < 0){
		out("error snarf %r");
		return;
	}
	n = readn(fd, buf, sizeof buf);
	close(fd);
	p = hex;
	for(i = 0; i < n; i++)
		p = seprint(p, hex + sizeof hex, "%02ux", (uchar)buf[i]);
	*p = 0;
	out("snarf %s", hex);
}

static void
copy(void)
{
	int fd;

	fd = open("/dev/snarf", OWRITE|OTRUNC);
	if(fd < 0){
		out("error snarf %r");
		return;
	}
	write(fd, "r9-probe-snarf", 14);
	close(fd);
	out("snarfed");
}

/*
 * Write the message m of /dev/kbd. Return its first rune, or
 * 0 when it has none.
 */
static Rune
kbdmsg(char *m)
{
	char line[512], *p, *e, *s;
	Rune r, first;

	e = line + sizeof line;
	p = seprint(line, e, "kbd %c", m[0]);
	first = 0;
	for(s = m+1; *s != 0; ){
		s += chartorune(&r, s);
		if(first == 0)
			first = r;
		p = seprint(p, e, " %x", r);
	}
	out("%s", line);
	return first;
}

/*
 * Write the pattern of the audio check to /dev/audio: sample k of
 * the 2*Audioframes samples is k*7919+12345, in 16 bits, little
 * endian. r9 looks for the same bytes in the WAV file of the
 * monitor.
 */
static void
play(void)
{
	uchar *b;
	ushort v;
	int fd, k, n;

	n = 4*Audioframes;
	b = malloc(n);
	if(b == nil){
		out("error audio %r");
		return;
	}
	for(k = 0; k < 2*Audioframes; k++){
		v = k*7919 + 12345;
		b[2*k] = v;
		b[2*k+1] = v>>8;
	}
	fd = open("/dev/audio", OWRITE);
	if(fd < 0){
		out("error audio %r");
		free(b);
		return;
	}
	if(write(fd, b, n) != n){
		out("error audio write %r");
		n = 0;
	}
	close(fd);
	free(b);
	out("audio %d", n);
}

void
stop(int pid)
{
	play();
	out("end");
	postnote(PNPROC, pid, "kill");
	exits(nil);
}

void
main(void)
{
	char buf[256], *m;
	int kfd, mfd, n, pid, ending, down;
	Rune r;

	cons = open("#c/cons", OWRITE);
	if(cons < 0)
		exits("cons");
	kfd = open("/dev/kbd", OREAD);
	if(kfd < 0){
		out("error kbd %r");
		exits("kbd");
	}
	mfd = open("/dev/mouse", OREAD);
	if(mfd < 0){
		out("error mouse %r");
		exits("mouse");
	}
	pid = rfork(RFPROC|RFMEM);
	if(pid < 0){
		out("error rfork %r");
		exits("rfork");
	}
	if(pid == 0)
		mouseproc(mfd);
	out("ready");
	ending = 0;
	down = 0;
	while((n = read(kfd, buf, sizeof buf - 1)) > 0){
		buf[n] = 0;
		for(m = buf; m < buf+n; m += strlen(m)+1){
			r = kbdmsg(m);
			if(m[0] != 'c'){
				/* the probe stops when the keys of Ctrl-D are up */
				down = r != 0;
				if(ending && !down)
					stop(pid);
				continue;
			}
			switch(r){
			case 0x16:	/* Ctrl-V */
				paste();
				break;
			case 0x03:	/* Ctrl-C */
				copy();
				break;
			case 0x04:	/* Ctrl-D */
				ending = 1;
				/* the keys can be up before the c message */
				if(!down)
					stop(pid);
				break;
			}
		}
	}
	out("error kbd read %r");
	postnote(PNPROC, pid, "kill");
	exits("kbd");
}
