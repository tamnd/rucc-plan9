/* dial: a child listens on a tcp port of the loopback, and the parent dials it. */
#include <u.h>
#include <libc.h>

void
main(void)
{
	char adir[40], ldir[40], buf[16];
	int afd, lfd, dfd, fd, n;

	afd = announce("tcp!*!17019", adir);
	if(afd < 0)
		sysfatal("announce: %r");
	switch(rfork(RFPROC|RFFDG)){
	case -1:
		sysfatal("rfork: %r");
	case 0:
		lfd = listen(adir, ldir);
		if(lfd < 0)
			sysfatal("listen: %r");
		dfd = accept(lfd, ldir);
		if(dfd < 0)
			sysfatal("accept: %r");
		write(dfd, "r9 dial", 7);
		exits(nil);
	}
	fd = dial("tcp!127.0.0.1!17019", nil, nil, nil);
	if(fd < 0)
		sysfatal("dial: %r");
	n = readn(fd, buf, 7);
	if(n != 7 || memcmp(buf, "r9 dial", 7) != 0)
		sysfatal("read %d bytes", n);
	if(waitpid() < 0)
		sysfatal("waitpid: %r");
	exits(nil);
}
