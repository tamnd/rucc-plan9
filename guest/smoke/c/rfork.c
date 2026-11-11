/*
 * rfork with each flag. The argument is the flag letter of rc's rfork, or w for RFNOWAIT and
 * r for RFREND. The child checks what the flag gives it and the parent checks what it keeps.
 */
#include <u.h>
#include <libc.h>

long shared;

static int
exists(char *f)
{
	return access(f, AEXIST) == 0;
}

static void
put(char *f, char *s)
{
	int fd;

	fd = create(f, OWRITE, 0666);
	if(fd < 0)
		sysfatal("create %s: %r", f);
	write(fd, s, strlen(s));
	close(fd);
}

static long
noteid(int pid)
{
	char f[64], buf[32];
	int fd, n;

	snprint(f, sizeof f, "/proc/%d/noteid", pid);
	fd = open(f, OREAD);
	if(fd < 0)
		sysfatal("open %s: %r", f);
	n = read(fd, buf, sizeof buf - 1);
	close(fd);
	if(n <= 0)
		sysfatal("read %s: %r", f);
	buf[n] = 0;
	return strtol(buf, nil, 10);
}

/* Wait for the child and give its status. */
static void
child(void)
{
	Waitmsg *w;

	w = wait();
	if(w == nil)
		sysfatal("wait: %r");
	if(w->msg[0] != 0)
		sysfatal("child: %s", w->msg);
	free(w);
}

void
main(int argc, char **argv)
{
	int fd, pid;
	long parent;
	void *v;
	char path[64];

	if(argc != 2)
		sysfatal("usage: rfork flag");
	switch(argv[1][0]){
	case 'n':
		/* A new copy of the name space: a bind in the child is not in the parent. */
		if(!exists("/tmp/r9t/b/f") || exists("/tmp/r9t/a/f"))
			sysfatal("set up /tmp/r9t first");
		if(rfork(RFPROC|RFNAMEG|RFFDG) == 0){
			if(bind("/tmp/r9t/b", "/tmp/r9t/a", MREPL) < 0)
				sysfatal("bind: %r");
			exits(exists("/tmp/r9t/a/f") ? nil : "no bind");
		}
		child();
		if(exists("/tmp/r9t/a/f"))
			sysfatal("the bind is in the parent");
		break;
	case 'N':
		/* A clean name space: the child has no /bin. */
		if(rfork(RFPROC|RFCNAMEG|RFFDG) == 0)
			_exits(exists("/bin/rc") ? "the child has /bin" : nil);
		child();
		break;
	case 'e':
		/* A new copy of the environment: a variable that the child sets is not in the parent. */
		remove("/env/r9e");
		if(rfork(RFPROC|RFENVG|RFFDG) == 0){
			put("/env/r9e", "1");
			exits(nil);
		}
		child();
		if(exists("/env/r9e"))
			sysfatal("the variable is in the parent");
		break;
	case 'E':
		/* A clean environment: the child does not see the variables of the parent. */
		put("/env/r9E", "1");
		if(rfork(RFPROC|RFCENVG|RFFDG) == 0)
			exits(exists("/env/r9E") ? "the child has the variable" : nil);
		child();
		remove("/env/r9E");
		break;
	case 's':
		/* A new note group. */
		parent = noteid(getpid());
		if(rfork(RFPROC|RFNOTEG|RFFDG) == 0)
			exits(noteid(getpid()) != parent ? nil : "the same note group");
		child();
		break;
	case 'f':
		/* A copy of the file descriptors: a close in the child does not close the parent's. */
		fd = open("/dev/null", OREAD);
		if(fd < 0)
			sysfatal("open: %r");
		if(rfork(RFPROC|RFFDG) == 0){
			close(fd);
			exits(nil);
		}
		child();
		if(fd2path(fd, path, sizeof path) < 0)
			sysfatal("the parent lost its descriptor");
		close(fd);
		break;
	case 'F':
		/* A clean file descriptor table. */
		fd = open("/dev/null", OREAD);
		if(rfork(RFPROC|RFCFDG) == 0)
			_exits(fd2path(fd, path, sizeof path) < 0 ? nil : "the child has the descriptor");
		child();
		close(fd);
		break;
	case 'm':
		/* Shared memory: what the child writes, the parent reads. */
		shared = 1;
		if(rfork(RFPROC|RFMEM|RFFDG) == 0){
			shared = 42;
			_exits(nil);
		}
		child();
		if(shared != 42)
			sysfatal("the parent reads %ld", shared);
		break;
	case 'r':
		/* A new rendezvous group: the child cannot meet the parent, and meets its own child. */
		if(rfork(RFPROC|RFREND|RFFDG) == 0){
			if(rfork(RFPROC|RFFDG) == 0){
				rendezvous((void*)0x51, (void*)5);
				exits(nil);
			}
			v = rendezvous((void*)0x51, (void*)6);
			child();
			exits(v == (void*)5 ? nil : "no rendezvous in the new group");
		}
		child();
		break;
	case 'w':
		/* No wait record: the parent has no child to wait for. */
		pid = rfork(RFPROC|RFNOWAIT|RFFDG);
		if(pid == 0)
			_exits(nil);
		if(pid < 0)
			sysfatal("rfork: %r");
		if(wait() != nil)
			sysfatal("a wait record for a child of RFNOWAIT");
		break;
	default:
		sysfatal("unknown flag %s", argv[1]);
	}
	exits(nil);
}
