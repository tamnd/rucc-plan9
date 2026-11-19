/*
 * r9gen: make the inputs of the benchmarks of plan 13.2. Each input comes from a fixed sequence of
 * numbers, so that it is the same on each run and on each image.
 *
 *	r9gen log FILE MB	lines of a log, MB MiB in all
 *	r9gen lines FILE N	N lines of words, for sort
 *	r9gen tree DIR MB	small files in directories, MB MiB in all, for tar
 *	r9gen big FILE MB	one file of MB MiB
 */
#include <u.h>
#include <libc.h>
#include <bio.h>

enum
{
	MiB = 1024*1024,
};

static ulong seed = 1;

static ulong
next(void)
{
	seed = seed*1103515245 + 12345;
	return seed>>8;
}

static char *words[] = {
	"rio", "acme", "sam", "plumber", "factotum", "exportfs", "ramfs", "gefs", "cwfs", "mk",
	"rc", "awk", "sed", "grep", "sort", "troff", "venti", "fossil", "kfs", "upas",
	"walk", "open", "read", "write", "clunk", "stat", "wstat", "attach", "auth", "flush",
};

static Biobuf*
create1(char *file)
{
	Biobuf *b;

	b = Bopen(file, OWRITE);
	if(b == nil)
		sysfatal("create %s: %r", file);
	return b;
}

static void
genlog(char *file, long mb)
{
	Biobuf *b;
	vlong n;
	ulong r;

	b = create1(file);
	n = 0;
	while(n < (vlong)mb*MiB){
		r = next();
		n += Bprint(b, "2026-11-%02uld %02uld:%02uld:%02uld %s[%uld]: %s %s of %uld bytes in %uld.%03uld ms\n",
			1 + r%28, r%24, (r>>5)%60, (r>>11)%60, words[r%nelem(words)], r%30000,
			words[(r>>3)%nelem(words)], words[(r>>7)%nelem(words)], next()%65536, (r>>9)%1000, r%1000);
	}
	Bterm(b);
}

static void
genlines(char *file, long count)
{
	Biobuf *b;
	long i;
	ulong r;

	b = create1(file);
	for(i = 0; i < count; i++){
		r = next();
		Bprint(b, "%s %08lux %s %uld\n", words[r%nelem(words)], next(), words[(r>>5)%nelem(words)], r%100000);
	}
	Bterm(b);
}

static void
gentree(char *dir, long mb)
{
	char path[256];
	uchar data[16384];
	vlong n;
	long i, size;
	int fd;

	for(i = 0; i < sizeof data; i++)
		data[i] = "abcdefghijklmnopqrstuvwxyz\n"[next()%27];
	n = 0;
	for(i = 0; n < (vlong)mb*MiB; i++){
		if(i%100 == 0){
			snprint(path, sizeof path, "%s/d%04ld", dir, i/100);
			fd = create(path, OREAD, DMDIR|0755);
			if(fd < 0)
				sysfatal("create %s: %r", path);
			close(fd);
		}
		size = 100 + next()%(sizeof data - 100);
		snprint(path, sizeof path, "%s/d%04ld/f%05ld", dir, i/100, i);
		fd = create(path, OWRITE, 0644);
		if(fd < 0)
			sysfatal("create %s: %r", path);
		if(write(fd, data, size) != size)
			sysfatal("write %s: %r", path);
		close(fd);
		n += size;
	}
}

static void
genbig(char *file, long mb)
{
	uchar data[65536];
	long i;
	int fd;

	for(i = 0; i < sizeof data; i++)
		data[i] = next();
	fd = create(file, OWRITE, 0644);
	if(fd < 0)
		sysfatal("create %s: %r", file);
	for(i = 0; i < mb*MiB; i += sizeof data)
		if(write(fd, data, sizeof data) != sizeof data)
			sysfatal("write %s: %r", file);
	close(fd);
}

void
main(int argc, char **argv)
{
	long n;

	if(argc != 4){
		fprint(2, "usage: r9gen log|lines|tree|big path n\n");
		exits("usage");
	}
	n = strtol(argv[3], nil, 10);
	if(strcmp(argv[1], "log") == 0)
		genlog(argv[2], n);
	else if(strcmp(argv[1], "lines") == 0)
		genlines(argv[2], n);
	else if(strcmp(argv[1], "tree") == 0)
		gentree(argv[2], n);
	else if(strcmp(argv[1], "big") == 0)
		genbig(argv[2], n);
	else
		sysfatal("no input %s", argv[1]);
	exits(nil);
}
