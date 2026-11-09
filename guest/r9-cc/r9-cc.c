/*
 * r9-cc: the compiler slot shim (plan 05.3), in recording mode.
 *
 * r9-build binds this program over 6c, 7c, 8c, 5c and vc, after it copies the real compilers
 * to /tmp/r9/bin. For each call, r9-cc runs the real compiler with the same arguments, and
 * appends one JSON record to /tmp/r9/compile.jsonl: the compiler, objtype, the directory, the
 * arguments, the SHA-256 of each input and output, the exit status and the time. The file is
 * append-only, so parallel calls from mk do not mix their records.
 *
 * The mode that sends the call to rucc on the host (r9 ccd, plan 05.4) comes at P1.
 */
#include <u.h>
#include <libc.h>
#include <libsec.h>

enum {
	Maxfiles = 256,
	Hexlen = 2*SHA2_256dlen+1,
};

char *realdir = "/tmp/r9/bin";
char *records = "/tmp/r9/compile.jsonl";

typedef struct Buf Buf;
struct Buf {
	char *p;
	int n;
	int cap;
};

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

void
main(int argc, char **argv)
{
	char *cc, *objtype, *real, *dir, *status, *origin;
	char *in[Maxfiles], *out[Maxfiles], hex[Hexlen];
	int i, nin, nout, pid, l;
	vlong t0, t1;
	Waitmsg *w;
	Buf b;

	cc = base(argv[0]);
	objtype = getenv("objtype");
	if(objtype == nil)
		objtype = "";
	real = smprint("%s/%s", realdir, cc);
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
	pid = fork();
	switch(pid){
	case -1:
		sysfatal("r9-cc: fork: %r");
	case 0:
		argv[0] = cc;
		exec(real, argv);
		sysfatal("r9-cc: exec %s: %r", real);
	}
	status = "";
	while((w = wait()) != nil){
		if(w->pid == pid){
			status = strdup(w->msg);
			free(w);
			break;
		}
		free(w);
	}
	t1 = nsec();

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
	origin = l == hostletter(objtype) ? "ref-c" : "other-arch";
	put(&b, ",\"origin\":");
	putq(&b, origin);
	put(&b, "}\n");
	record(&b);

	if(status[0] != 0)
		exits(status);
	exits(nil);
}
