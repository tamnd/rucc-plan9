/*
 * r9bench: the benchmarks of plan 13.2 that are C code. Each benchmark does a fixed number of
 * operations and prints one line:
 *
 *	bench NAME NS OPS
 *
 * NS is the time of all the operations in nanoseconds, from nsec. With no arguments, r9bench runs
 * each benchmark. With arguments, it runs the benchmarks with these names. A benchmark that
 * cannot run prints "skip NAME: reason". The work of each benchmark is the same on each run, so
 * that the runs of R and G can be compared.
 */
#include <u.h>
#include <libc.h>
#include <draw.h>
#include <memdraw.h>
#include <regexp.h>
#include <mp.h>
#include <libsec.h>

typedef struct Bench Bench;
struct Bench
{
	char	*name;
	long	ops;
	void	(*fn)(long);
};

enum
{
	MiB = 1024*1024,
	Page = 4096,
};

static char *skipped;
static vlong start;
static uchar buf[64*1024];

static void
skip(char *why)
{
	skipped = why;
}

/* A fixed sequence of numbers, so that each run does the same work. */
static ulong seed = 1;

static ulong
next(void)
{
	seed = seed*1103515245 + 12345;
	return seed>>8;
}

/* System calls. */

static void
berrstr(long n)
{
	char e[ERRMAX];

	while(n-- > 0){
		strcpy(e, "r9bench");
		errstr(e, sizeof e);
	}
}

static void
bpipe1(long n)
{
	int p[2];
	char c;

	if(pipe(p) < 0){
		skip("pipe");
		return;
	}
	c = 'x';
	while(n-- > 0){
		write(p[0], &c, 1);
		read(p[1], &c, 1);
	}
	close(p[0]);
	close(p[1]);
}

static void
bopen(long n)
{
	int fd;

	while(n-- > 0){
		fd = open("/dev/null", OREAD);
		if(fd < 0){
			skip("open /dev/null");
			return;
		}
		close(fd);
	}
}

static void
bdirstat(long n)
{
	Dir *d;

	while(n-- > 0){
		d = dirstat("/bin/rc");
		if(d == nil){
			skip("dirstat /bin/rc");
			return;
		}
		free(d);
	}
}

static void
brendez(long n)
{
	long i;

	switch(rfork(RFPROC|RFMEM|RFFDG)){
	case -1:
		skip("rfork");
		return;
	case 0:
		for(i = 0; i < n; i++)
			rendezvous((void*)1, (void*)i);
		_exits(nil);
	}
	for(i = 0; i < n; i++)
		rendezvous((void*)1, (void*)i);
	waitpid();
}

/* Processes. */

static void
brforkmem(long n)
{
	while(n-- > 0){
		switch(rfork(RFPROC|RFMEM|RFFDG)){
		case -1:
			skip("rfork");
			return;
		case 0:
			_exits(nil);
		}
		waitpid();
	}
}

static void
bexec(long n)
{
	int fd;

	while(n-- > 0){
		switch(rfork(RFPROC|RFFDG)){
		case -1:
			skip("rfork");
			return;
		case 0:
			fd = open("/dev/null", OWRITE);
			dup(fd, 1);
			execl("/bin/echo", "echo", nil);
			_exits("exec");
		}
		waitpid();
	}
}

static void
bswitch(long n)
{
	int a[2], b[2];
	long i;
	char c;

	if(pipe(a) < 0 || pipe(b) < 0){
		skip("pipe");
		return;
	}
	switch(rfork(RFPROC|RFFDG)){
	case -1:
		skip("rfork");
		return;
	case 0:
		for(i = 0; i < n; i++){
			read(a[1], &c, 1);
			write(b[1], &c, 1);
		}
		_exits(nil);
	}
	c = 'x';
	for(i = 0; i < n; i++){
		write(a[0], &c, 1);
		read(b[0], &c, 1);
	}
	waitpid();
	close(a[0]);
	close(a[1]);
	close(b[0]);
	close(b[1]);
}

static long sems[2];

static void
bsem(long n)
{
	long i;

	sems[0] = sems[1] = 0;
	switch(rfork(RFPROC|RFMEM|RFFDG)){
	case -1:
		skip("rfork");
		return;
	case 0:
		for(i = 0; i < n; i++){
			semacquire(&sems[0], 1);
			semrelease(&sems[1], 1);
		}
		_exits(nil);
	}
	for(i = 0; i < n; i++){
		semrelease(&sems[0], 1);
		semacquire(&sems[1], 1);
	}
	waitpid();
}

/* Memory. */

static void
bfault(long n)
{
	uchar *p;
	ulong i;

	while(n-- > 0){
		switch(rfork(RFPROC|RFFDG)){
		case -1:
			skip("rfork");
			return;
		case 0:
			p = sbrk(256*MiB);
			if(p == (void*)-1)
				_exits("sbrk");
			for(i = 0; i < 256*MiB; i += Page)
				p[i] = i;
			_exits(nil);
		}
		waitpid();
	}
}

static void
bsegattach(long n)
{
	uchar *p;

	while(n-- > 0){
		p = segattach(0, "memory", nil, MiB);
		if(p == (void*)-1){
			skip("segattach");
			return;
		}
		p[0] = 1;
		p[MiB-1] = 1;
		segdetach(p);
	}
}

static void
bmalloc(long n)
{
	void *slot[256];
	ulong r, size;
	int i;

	memset(slot, 0, sizeof slot);
	while(n-- > 0){
		r = next();
		i = r & 255;
		free(slot[i]);
		/* Mostly small blocks, and sometimes a large one, up to 1 MiB. */
		size = 16 << ((r>>8) % 7);
		if(((r>>12) & 63) == 0)
			size = 16 << ((r>>16) % 17);
		slot[i] = malloc(size);
		if(slot[i] == nil){
			skip("malloc");
			return;
		}
		((uchar*)slot[i])[0] = r;
	}
	for(i = 0; i < 256; i++)
		free(slot[i]);
}

/* Files. */

static void
bramfs(long n)
{
	int fd;
	long i;

	if(access("/n/r9ram", AEXIST) < 0){
		skip("no ramfs at /n/r9ram");
		return;
	}
	while(n-- > 0){
		fd = create("/n/r9ram/f", ORDWR, 0644);
		if(fd < 0){
			skip("create in ramfs");
			return;
		}
		for(i = 0; i < 100*MiB; i += sizeof buf)
			write(fd, buf, sizeof buf);
		seek(fd, 0, 0);
		while(read(fd, buf, sizeof buf) > 0)
			;
		close(fd);
		remove("/n/r9ram/f");
	}
}

static void
bexportfs(long n)
{
	int p[2], fd;

	if(access("/tmp/r9bench/big", AREAD) < 0){
		skip("no /tmp/r9bench/big");
		return;
	}
	while(n-- > 0){
		if(pipe(p) < 0){
			skip("pipe");
			return;
		}
		switch(rfork(RFPROC|RFFDG|RFNOTEG)){
		case -1:
			skip("rfork");
			return;
		case 0:
			dup(p[1], 0);
			dup(p[1], 1);
			close(p[0]);
			close(p[1]);
			execl("/bin/exportfs", "exportfs", "-r", "/tmp/r9bench", nil);
			_exits("exec");
		}
		close(p[1]);
		if(mount(p[0], -1, "/n/r9ex", MREPL, "") < 0){
			skip("mount of exportfs");
			return;
		}
		fd = open("/n/r9ex/big", OREAD);
		if(fd < 0){
			skip("open through exportfs");
			return;
		}
		while(read(fd, buf, sizeof buf) > 0)
			;
		close(fd);
		unmount(nil, "/n/r9ex");
		close(p[0]);
		waitpid();
	}
}

/* Network. */

static void
btcp(long n)
{
	char adir[40], ldir[40];
	int afd, lfd, dfd, fd;
	long i, total;

	afd = announce("tcp!*!5700", adir);
	if(afd < 0){
		skip("announce tcp!*!5700");
		return;
	}
	switch(rfork(RFPROC|RFFDG)){
	case -1:
		skip("rfork");
		return;
	case 0:
		fd = dial("tcp!127.0.0.1!5700", nil, nil, nil);
		if(fd < 0)
			_exits("dial");
		for(i = 0; i < n; i++)
			write(fd, buf, sizeof buf);
		close(fd);
		_exits(nil);
	}
	lfd = listen(adir, ldir);
	if(lfd < 0){
		skip("listen");
		return;
	}
	dfd = accept(lfd, ldir);
	total = 0;
	while((i = read(dfd, buf, sizeof buf)) > 0)
		total += i;
	close(dfd);
	close(lfd);
	close(afd);
	waitpid();
	if(total != n*(long)sizeof buf)
		skip("short read on tcp");
}

static void
budp(long n)
{
	int rfd, fd, pid;
	long i;

	/* Two conversations that are connected to each other on the loopback address. */
	rfd = dial("udp!127.0.0.1!5702", "127.0.0.1!5701", nil, nil);
	if(rfd < 0){
		skip("dial udp!127.0.0.1!5702");
		return;
	}
	fd = dial("udp!127.0.0.1!5701", "127.0.0.1!5702", nil, nil);
	if(fd < 0){
		skip("dial udp!127.0.0.1!5701");
		return;
	}
	pid = rfork(RFPROC|RFFDG);
	switch(pid){
	case -1:
		skip("rfork");
		return;
	case 0:
		/* The reader takes the packets that come, until the writer kills it. */
		close(fd);
		alarm(60*1000);
		while(read(rfd, buf, sizeof buf) > 0)
			;
		_exits(nil);
	}
	close(rfd);
	for(i = 0; i < n; i++)
		write(fd, buf, 64);
	close(fd);
	postnote(PNPROC, pid, "kill");
	waitpid();
}

/* Libraries. */

static int
cmplong(void *a, void *b)
{
	long x, y;

	x = *(long*)a;
	y = *(long*)b;
	return x < y ? -1 : x > y;
}

static void
bqsort(long n)
{
	long *v;
	long i, k;

	k = 100000;
	v = malloc(k*sizeof v[0]);
	if(v == nil){
		skip("malloc");
		return;
	}
	while(n-- > 0){
		for(i = 0; i < k; i++)
			v[i] = next();
		qsort(v, k, sizeof v[0], cmplong);
	}
	free(v);
}

static void
bstrtod(long n)
{
	static char *s[] = {
		"0", "1", "3.14159265358979", "-2.718281828e-10", "6.02214076e23",
		"1e308", "4.9e-324", "123456789.123456789", "0.1", "1.7976931348623157e308",
	};
	double d;

	d = 0;
	while(n-- > 0)
		d += strtod(s[n%nelem(s)], nil);
	USED(d);
}

static void
bprintg(long n)
{
	char s[64];
	double d;

	d = 1.0;
	while(n-- > 0){
		snprint(s, sizeof s, "%g", d);
		d = d*1.0001 + 0.37;
	}
}

static void
bregexp(long n)
{
	Reprog *p;
	Resub m[4];
	static char *text[] = {
		"the quick brown fox jumps over the lazy dog",
		"GET /sys/src/cmd/rc/exec.c HTTP/1.1",
		"2026-11-18 12:21:36 r9bench: 51 users, load average: 16.26",
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab",
	};
	int i;

	while(n-- > 0){
		p = regcomp("([a-z]+)/([a-z0-9]+)\\.c|load average: ([0-9.]+)|a*b$");
		if(p == nil){
			skip("regcomp");
			return;
		}
		for(i = 0; i < nelem(text); i++){
			memset(m, 0, sizeof m);
			regexec(p, text[i], m, nelem(m));
		}
		free(p);
	}
}

static void
bmemmove(long n)
{
	static int size[] = { 1, 3, 8, 15, 32, 63, 128, 255, 1024, 4095, 16384, 65536 };
	static uchar a[65536+64], b[65536+64];
	int i;

	while(n-- > 0)
		for(i = 0; i < nelem(size); i++){
			memmove(a + (n&7), b + (i&7), size[i]);
			memmove(b + 1, b, size[i]);
		}
}

static void
brsa(long n)
{
	RSApriv *k;
	mpint *m, *s;

	k = rsagen(2048, 17, 0);
	if(k == nil){
		skip("rsagen");
		return;
	}
	m = strtomp("123456789abcdef0123456789abcdef0123456789abcdef", nil, 16, nil);
	s = mpnew(0);
	/* The key is random, so the time to make it is not part of the benchmark. */
	start = nsec();
	while(n-- > 0)
		rsadecrypt(k, m, s);
	mpfree(m);
	mpfree(s);
	rsaprivfree(k);
}

static void
bsha256(long n)
{
	uchar d[SHA2_256dlen];
	DigestState *s;
	long i;

	/* One operation is one buffer of 64 KiB. */
	s = nil;
	for(i = 0; i < n; i++)
		s = sha2_256(buf, sizeof buf, nil, s);
	sha2_256(nil, 0, d, s);
}

static void
baes(long n)
{
	AESstate s;
	uchar key[16], iv[16];
	long i;

	memset(key, 7, sizeof key);
	memset(iv, 9, sizeof iv);
	/* One operation is one buffer of 64 KiB. */
	setupAESstate(&s, key, sizeof key, iv);
	for(i = 0; i < n; i++)
		aesCBCencrypt(buf, sizeof buf, &s);
}

/* Graphics. */

static void
bmemdraw(long n)
{
	Memimage *dst, *src, *mask;
	Rectangle r;
	int x, y, w, h, op;
	ulong v;

	if(memimageinit() < 0){
		skip("memimageinit");
		return;
	}
	dst = allocmemimage(Rect(0, 0, 1024, 768), XRGB32);
	src = allocmemimage(Rect(0, 0, 256, 256), ARGB32);
	mask = allocmemimage(Rect(0, 0, 256, 256), GREY8);
	if(dst == nil || src == nil || mask == nil){
		skip("allocmemimage");
		return;
	}
	memfillcolor(dst, DWhite);
	memfillcolor(src, 0x3366CC80);
	memfillcolor(mask, 0x80808080);
	while(n-- > 0){
		v = next();
		w = 1 + v % 256;
		h = 1 + (v >> 8) % 256;
		x = (v >> 16) % (1024 - w);
		y = next() % (768 - h);
		r = Rect(x, y, x + w, y + h);
		op = (v >> 24) & 1 ? SoverD : S;
		if((v >> 25) & 1)
			memimagedraw(dst, r, src, ZP, mask, ZP, op);
		else
			memimagedraw(dst, r, src, ZP, nil, ZP, op);
	}
	freememimage(dst);
	freememimage(src);
	freememimage(mask);
}

static Bench benches[] = {
	{ "errstr", 2000000, berrstr },
	{ "pipe1", 400000, bpipe1 },
	{ "open", 30000, bopen },
	{ "dirstat", 10000, bdirstat },
	{ "rendezvous", 500000, brendez },
	{ "rforkmem", 20000, brforkmem },
	{ "exec", 2000, bexec },
	{ "switch", 100000, bswitch },
	{ "sem", 200000, bsem },
	{ "fault", 4, bfault },
	{ "segattach", 40000, bsegattach },
	{ "malloc", 1000000, bmalloc },
	{ "ramfs", 2, bramfs },
	{ "qsort", 20, bqsort },
	{ "strtod", 10000, bstrtod },
	{ "printg", 30000, bprintg },
	{ "regexp", 20000, bregexp },
	{ "memmove", 50000, bmemmove },
	{ "rsa", 200, brsa },
	{ "sha256", 1024, bsha256 },
	{ "aes", 512, baes },
	{ "memdraw", 2000, bmemdraw },
	{ "exportfs", 1, bexportfs },
	{ "tcp", 32768, btcp },
	{ "udp", 200000, budp },
};

static void
run(Bench *b)
{
	vlong t;

	skipped = nil;
	seed = 1;
	start = nsec();
	b->fn(b->ops);
	t = nsec() - start;
	if(skipped != nil)
		print("skip %s: %s: %r\n", b->name, skipped);
	else
		print("bench %s %lld %ld\n", b->name, t, b->ops);
}

/*
 * Run a command with its standard output on /dev/null, and print its time as a benchmark of one
 * operation. The command fails the benchmark when its status is not empty.
 */
static void
timed(char *name, char **argv)
{
	Waitmsg *w;
	vlong t;
	int fd;

	t = nsec();
	switch(rfork(RFPROC|RFFDG)){
	case -1:
		print("skip %s: rfork: %r\n", name);
		return;
	case 0:
		fd = open("/dev/null", OWRITE);
		dup(fd, 1);
		exec(argv[0], argv);
		_exits("exec");
	}
	w = wait();
	t = nsec() - t;
	if(w == nil)
		print("skip %s: wait: %r\n", name);
	else if(w->msg[0] != 0)
		print("skip %s: %s\n", name, w->msg);
	else
		print("bench %s %lld 1\n", name, t);
	free(w);
}

static void
usage(void)
{
	fprint(2, "usage: r9bench [name ...]\n       r9bench -x name command [arg ...]\n");
	exits("usage");
}

void
main(int argc, char **argv)
{
	char *x;
	int i, j;

	x = nil;
	ARGBEGIN{
	case 'x':
		x = EARGF(usage());
		break;
	default:
		usage();
	}ARGEND
	if(x != nil){
		if(argc == 0)
			usage();
		timed(x, argv);
		exits(nil);
	}
	for(i = 0; i < nelem(benches); i++){
		if(argc == 0){
			run(&benches[i]);
			continue;
		}
		for(j = 0; j < argc; j++)
			if(strcmp(argv[j], benches[i].name) == 0)
				run(&benches[i]);
	}
	exits(nil);
}
