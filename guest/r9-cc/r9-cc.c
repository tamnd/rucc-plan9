/*
 * r9-cc: the compiler slot shim (plan 05.3), in recording mode.
 *
 * r9-build puts a copy of this program for each of 6c, 7c, 8c, 5c and vc in a directory that
 * it binds before /bin. For each call, r9-cc runs the real compiler, /$objtype/bin/<name>, with
 * the same arguments. Then it appends one JSON record to /tmp/r9/compile.jsonl: the compiler,
 * objtype, the directory, the arguments, the SHA-256 of each input and output, the exit status
 * and the time. The file is append-only, so parallel calls from mk do not mix their records.
 *
 * When $r9ccd names the host, such as tcp!10.0.2.2!5640, a call for the machine of the guest
 * goes to rucc through r9 ccd (plan 05.4). r9-cc dials the host two times. On the first
 * connection it runs exportfs, so that rucc reads the sources and the headers from the name
 * space of the build. On the second it sends the call, and it writes the objects of the reply.
 * The record then has the origin rucc. When rucc fails and $r9bringup has cc:auto, r9-cc runs
 * the real compiler, and the record has the origin ref-c.
 *
 * r9-build also puts a copy for each of 6a, 7a, 8a, 5a and va (plan 07.5). An assembler call
 * goes to r9 ccd the same way. When the real assembler of the guest runs, the record has the
 * origin ref-as.
 *
 * The record also has what the compiler that made the object wrote to standard error, so that
 * r9 warn-diff compares the warnings of G and R for each unit (plan 05.8). r9-cc still writes
 * the text to its own standard error, so mk.log does not change.
 */
#include <u.h>
#include <libc.h>
#include <bio.h>
#include <libsec.h>

enum {
	Maxfiles = 256,
	Hexlen = 2*SHA2_256dlen+1,
	Tries = 3,	/* calls to r9 ccd before a lost call goes to kencc */
};

char *records = "/tmp/r9/compile.jsonl";
char incomplete[] = "r9-cc: the reply of r9 ccd is not complete";

typedef struct Buf Buf;
struct Buf {
	char *p;
	int n;
	int cap;
};

/* What the compiler wrote to standard error, for the record. */
Buf errs;

void keep(char*, long);

void
grow(Buf *b, int n)
{
	if(b->n+n < b->cap)
		return;
	b->cap = (b->n+n)*2 + 256;
	b->p = realloc(b->p, b->cap);
	if(b->p == nil)
		sysfatal("r9-cc: out of memory");
}

void
put(Buf *b, char *s)
{
	int n;

	n = strlen(s);
	grow(b, n);
	memmove(b->p+b->n, s, n);
	b->n += n;
	b->p[b->n] = 0;
}

void
putq(Buf *b, char *s)
{
	char e[8];

	put(b, "\"");
	for(; *s; s++){
		switch(*s){
		case '"':
			put(b, "\\\"");
			break;
		case '\\':
			put(b, "\\\\");
			break;
		case '\n':
			put(b, "\\n");
			break;
		case '\t':
			put(b, "\\t");
			break;
		default:
			if((uchar)*s < 0x20){
				snprint(e, sizeof e, "\\u%04ux", (uchar)*s);
				put(b, e);
			}else{
				e[0] = *s;
				e[1] = 0;
				put(b, e);
			}
		}
	}
	put(b, "\"");
}

/* The SHA-256 of a file in hex, or "" when it cannot be read. */
void
hashfile(char *path, char *hex)
{
	DigestState *s;
	uchar buf[8192], d[SHA2_256dlen];
	int fd, n, i;

	hex[0] = 0;
	fd = open(path, OREAD);
	if(fd < 0)
		return;
	s = nil;
	while((n = read(fd, buf, sizeof buf)) > 0)
		s = sha2_256(buf, n, nil, s);
	close(fd);
	if(n < 0){
		free(s);
		return;
	}
	sha2_256(nil, 0, d, s);
	for(i = 0; i < SHA2_256dlen; i++)
		snprint(hex+2*i, 3, "%.2ux", d[i]);
}

/* The base name of a path. */
char*
base(char *p)
{
	char *s;

	s = strrchr(p, '/');
	return s ? s+1 : p;
}

/* The objtype letter of a compiler name, such as '6' for 6c. */
int
letter(char *cc)
{
	return cc[0];
}

/* Whether cc is the name of an assembler, such as 6a. */
int
assembler(char *cc)
{
	return cc[0] != 0 && strcmp(cc+1, "a") == 0;
}

/* The letter of the objtype of this guest. */
int
hostletter(char *objtype)
{
	if(strcmp(objtype, "amd64") == 0)
		return '6';
	if(strcmp(objtype, "arm64") == 0)
		return '7';
	if(strcmp(objtype, "386") == 0)
		return '8';
	if(strcmp(objtype, "arm") == 0)
		return '5';
	if(strcmp(objtype, "mips") == 0)
		return 'v';
	return 0;
}

/*
 * Find the inputs and the outputs from the arguments, as kencc does: each argument that does
 * not start with '-' is a source file, -o names the output, and with no -o a source x.c gives
 * x.L in the current directory.
 */
int
files(int argc, char **argv, int l, char **in, char **out, int *nin)
{
	char *o, *s, *dot;
	int i, nout;

	o = nil;
	*nin = 0;
	for(i = 1; i < argc; i++){
		if(argv[i][0] == '-'){
			if(strcmp(argv[i], "-o") == 0 && i+1 < argc)
				o = argv[++i];
			else if(strncmp(argv[i], "-o", 2) == 0 && argv[i][2] != 0)
				o = argv[i]+2;
			else if((argv[i][1] == 'I' || argv[i][1] == 'D') && argv[i][2] == 0 && i+1 < argc)
				i++;
			continue;
		}
		if(*nin < Maxfiles)
			in[(*nin)++] = argv[i];
	}
	nout = 0;
	if(o != nil){
		out[nout++] = o;
		return nout;
	}
	for(i = 0; i < *nin && nout < Maxfiles; i++){
		s = smprint("%s", base(in[i]));
		dot = strrchr(s, '.');
		if(dot != nil)
			*dot = 0;
		out[nout++] = smprint("%s.%c", s, l);
		free(s);
	}
	return nout;
}

void
record(Buf *b)
{
	int fd;

	fd = open(records, OWRITE);
	if(fd < 0){
		fd = create(records, OWRITE|OEXCL, DMAPPEND|0666);
		if(fd < 0)
			fd = open(records, OWRITE);
	}
	if(fd < 0){
		fprint(2, "r9-cc: cannot open %s: %r\n", records);
		return;
	}
	if(write(fd, b->p, b->n) != b->n)
		fprint(2, "r9-cc: cannot write %s: %r\n", records);
	close(fd);
}

/* All the bytes of fd, in a buffer that ends in 0. */
char*
readall(int fd, long *len)
{
	Buf b;
	char tmp[8192];
	long n;

	memset(&b, 0, sizeof b);
	grow(&b, 1);
	while((n = read(fd, tmp, sizeof tmp)) > 0){
		grow(&b, n);
		memmove(b.p+b.n, tmp, n);
		b.n += n;
		b.p[b.n] = 0;
	}
	*len = b.n;
	return b.p;
}

/*
 * Run the real compiler with argv. When in is not nil, it is the standard input of the
 * compiler. Return the exit status and set the times.
 */
char*
runreal(char *real, char *cc, char **argv, char *in, long nin, ulong *user, ulong *sys)
{
	int pid, p[2], e[2];
	char *status, tmp[8192];
	long n;
	Waitmsg *w;

	if(in != nil && pipe(p) < 0)
		sysfatal("r9-cc: pipe: %r");
	if(pipe(e) < 0)
		sysfatal("r9-cc: pipe: %r");
	pid = fork();
	switch(pid){
	case -1:
		sysfatal("r9-cc: fork: %r");
	case 0:
		if(in != nil){
			dup(p[0], 0);
			close(p[0]);
			close(p[1]);
		}
		dup(e[1], 2);
		close(e[0]);
		close(e[1]);
		argv[0] = cc;
		exec(real, argv);
		sysfatal("r9-cc: exec %s: %r", real);
	}
	close(e[1]);
	if(in != nil){
		close(p[0]);
		write(p[1], in, nin);
		close(p[1]);
	}
	while((n = read(e[0], tmp, sizeof tmp)) > 0){
		write(2, tmp, n);
		keep(tmp, n);
	}
	close(e[0]);
	status = "";
	*user = *sys = 0;
	while((w = wait()) != nil){
		if(w->pid == pid){
			status = strdup(w->msg);
			*user = w->time[0];
			*sys = w->time[1];
			free(w);
			break;
		}
		free(w);
	}
	return status;
}

/* Read a line of the reply that starts with key and a space, and return the rest. */
char*
field(Biobuf *b, char *key)
{
	char *l;
	int n;

	l = Brdstr(b, '\n', 1);
	if(l == nil)
		return nil;
	n = strlen(key);
	if(strncmp(l, key, n) != 0 || (l[n] != ' ' && l[n] != 0)){
		free(l);
		return nil;
	}
	return l[n] == 0 ? l+n : l+n+1;
}

/* Keep n bytes of standard error for the record. */
void
keep(char *p, long n)
{
	grow(&errs, n+1);
	memmove(errs.p+errs.n, p, n);
	errs.n += n;
	errs.p[errs.n] = 0;
}

/*
 * Copy n bytes of the reply to fd, or skip them when fd is negative. With save set, keep them
 * for the record too.
 */
int
copyout(Biobuf *b, int fd, long n, int save)
{
	char tmp[8192];
	long m;

	while(n > 0){
		m = n < sizeof tmp ? n : sizeof tmp;
		if(Bread(b, tmp, m) != m)
			return -1;
		if(fd >= 0 && write(fd, tmp, m) != m)
			return -1;
		if(save)
			keep(tmp, m);
		n -= m;
	}
	return 0;
}

/*
 * Send the call to r9 ccd at addr. Return 0 when the host gave a reply, with its status in
 * *status, or -1 with the reason in *status. Standard error goes to errfd. The files of a
 * reply with an empty status are written in the guest.
 */
int
forward(char *addr, char *cc, char *objtype, char *dir, int argc, char **argv, char *in, long nin, int errfd, char **status)
{
	char *id, *v;
	int efd, cfd, fd, i;
	long n;
	Biobuf b;

	id = smprint("%d.%lld", getpid(), nsec());
	efd = dial(addr, nil, nil, nil);
	if(efd < 0){
		*status = smprint("r9-cc: dial %s: %r", addr);
		return -1;
	}
	fprint(efd, "9p %s\n", id);
	switch(rfork(RFPROC|RFFDG|RFNOWAIT)){
	case -1:
		*status = smprint("r9-cc: fork: %r");
		close(efd);
		return -1;
	case 0:
		dup(efd, 0);
		dup(efd, 1);
		close(efd);
		execl("/bin/exportfs", "exportfs", "-r", "/", nil);
		sysfatal("r9-cc: exec exportfs: %r");
	}
	close(efd);

	cfd = dial(addr, nil, nil, nil);
	if(cfd < 0){
		*status = smprint("r9-cc: dial %s: %r", addr);
		return -1;
	}
	Binit(&b, cfd, OWRITE);
	Bprint(&b, "call %s\ncc %s\nobjtype %s\ndir %s\nargc %d\n", id, cc, objtype, dir, argc-1);
	for(i = 1; i < argc; i++)
		Bprint(&b, "%s\n", argv[i]);
	Bprint(&b, "stdin %ld\n", in != nil ? nin : 0);
	if(in != nil)
		Bwrite(&b, in, nin);
	Bterm(&b);

	Binit(&b, cfd, OREAD);
	*status = incomplete;
	if((v = field(&b, "status")) == nil)
		goto out;
	*status = strdup(v);
	if((v = field(&b, "stderr")) == nil || copyout(&b, errfd, atol(v), 1) < 0)
		goto bad;
	for(;;){
		if((v = Brdstr(&b, '\n', 1)) == nil)
			goto bad;
		if(strcmp(v, "end") == 0)
			break;
		if(strncmp(v, "file ", 5) != 0)
			goto bad;
		n = strtol(v+5, &v, 10);
		if(*v++ != ' ')
			goto bad;
		fd = -1;
		if((*status)[0] == 0){
			fd = create(v, OWRITE, 0664);
			if(fd < 0){
				*status = smprint("r9-cc: create %s: %r", v);
				goto out;
			}
		}
		i = copyout(&b, fd, n, 0);
		if(fd >= 0)
			close(fd);
		if(i < 0)
			goto bad;
	}
	Bterm(&b);
	close(cfd);
	return 0;
bad:
	*status = incomplete;
out:
	Bterm(&b);
	close(cfd);
	return -1;
}

/*
 * Whether status is a call that did not get to r9 ccd or whose reply did not come back. Such a
 * call is tried again: a guest that the host stops for a while can lose the reply of a call that
 * rucc compiled, and kencc must not compile a unit for that reason.
 */
int
lost(char *status)
{
	return strcmp(status, incomplete) == 0 || strncmp(status, "r9-cc: dial ", 12) == 0;
}

void
main(int argc, char **argv)
{
	char *cc, *objtype, *real, *dir, *status, *origin, *ccd, *bringup, *text, *log;
	char *in[Maxfiles], *out[Maxfiles], hex[Hexlen];
	int i, nin, nout, l, fd;
	long ntext;
	vlong t0, t1;
	ulong user, sys;
	Buf b;

	cc = base(argv[0]);
	objtype = getenv("objtype");
	if(objtype == nil)
		objtype = "";
	real = smprint("/%s/bin/%s", objtype, cc);
	dir = getwd(malloc(1024), 1024);
	if(dir == nil)
		dir = "";
	l = letter(cc);
	nout = files(argc, argv, l, in, out, &nin);

	memset(&b, 0, sizeof b);
	put(&b, "{\"cc\":");
	putq(&b, cc);
	put(&b, ",\"objtype\":");
	putq(&b, objtype);
	put(&b, ",\"dir\":");
	putq(&b, dir);
	put(&b, ",\"argv\":[");
	for(i = 1; i < argc; i++){
		if(i > 1)
			put(&b, ",");
		putq(&b, argv[i]);
	}
	put(&b, "],\"in\":[");
	for(i = 0; i < nin; i++){
		hashfile(in[i], hex);
		if(i > 0)
			put(&b, ",");
		put(&b, "{\"file\":");
		putq(&b, in[i]);
		put(&b, ",\"sha256\":");
		putq(&b, hex);
		put(&b, "}");
	}
	put(&b, "]");

	t0 = nsec();
	ccd = getenv("r9ccd");
	bringup = getenv("r9bringup");
	if(l != hostletter(objtype))
		origin = "other-arch";
	else if(assembler(cc))
		origin = "ref-as";
	else
		origin = "ref-c";
	text = nil;
	ntext = 0;
	user = sys = 0;
	if(ccd != nil && ccd[0] != 0 && l == hostletter(objtype)){
		/* an assembler does not read its standard input */
		if(nin == 0 && !assembler(cc))
			text = readall(0, &ntext);
		log = "/tmp/r9/rucc.err";
		fd = open(log, OWRITE);
		if(fd < 0)
			fd = create(log, OWRITE, DMAPPEND|0666);
		if(bringup == nil || strstr(bringup, "cc:auto") == nil){
			close(fd);
			fd = 2;
		}else
			fprint(fd, "== %s %s\n", cc, dir);
		for(i = 0; i < Tries; i++){
			if(forward(ccd, cc, objtype, dir, argc, argv, text, ntext, fd, &status) == 0)
				break;
			if(!lost(status))
				break;
			fprint(fd, "%s; try %d of %d\n", status, i+1, Tries);
		}
		if(i < Tries && status[0] == 0)
			origin = "rucc";
		else if(fd != 2){
			fprint(fd, "%s\n", status);
			errs.n = 0;
			status = runreal(real, cc, argv, text, ntext, &user, &sys);
		}else{
			fprint(2, "%s\n", status);
			origin = "rucc";
		}
		if(fd != 2)
			close(fd);
	}else
		status = runreal(real, cc, argv, nil, 0, &user, &sys);
	t1 = nsec();
	/* nsec is the time of day, and the clock of the guest can step back */
	if(t1 < t0)
		t1 = t0;

	put(&b, ",\"out\":[");
	for(i = 0; i < nout; i++){
		hashfile(out[i], hex);
		if(i > 0)
			put(&b, ",");
		put(&b, "{\"file\":");
		putq(&b, out[i]);
		put(&b, ",\"sha256\":");
		putq(&b, hex);
		put(&b, "}");
	}
	put(&b, "],\"stdin\":");
	put(&b, nin == 0 ? "true" : "false");
	put(&b, ",\"status\":");
	putq(&b, status);
	put(&b, ",\"ms\":");
	snprint(hex, sizeof hex, "%lld", (t1-t0)/1000000);
	put(&b, hex);
	snprint(hex, sizeof hex, ",\"user_ms\":%lud,\"sys_ms\":%lud", user, sys);
	put(&b, hex);
	put(&b, ",\"origin\":");
	putq(&b, origin);
	put(&b, ",\"stderr\":");
	putq(&b, errs.n > 0 ? errs.p : "");
	put(&b, "}\n");
	record(&b);

	if(status[0] != 0)
		exits(status);
	exits(nil);
}
