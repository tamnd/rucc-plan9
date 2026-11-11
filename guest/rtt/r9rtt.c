/*
 * r9rtt: the guest side of r9 rtt (open question 10). Dial the host,
 * send one line, and serve the name space of the guest with exportfs on
 * the connection. The host times the 9P requests.
 *
 * usage: r9rtt [addr]
 */
#include <u.h>
#include <libc.h>

void
main(int argc, char **argv)
{
	char *addr;
	int fd;

	addr = "tcp!10.0.2.2!5641";
	if(argc > 1)
		addr = argv[1];
	fd = dial(addr, nil, nil, nil);
	if(fd < 0)
		sysfatal("dial %s: %r", addr);
	fprint(fd, "9p rtt\n");
	dup(fd, 0);
	dup(fd, 1);
	close(fd);
	execl("/bin/exportfs", "exportfs", "-r", "/", nil);
	sysfatal("exec exportfs: %r");
}
