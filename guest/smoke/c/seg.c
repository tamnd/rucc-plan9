/* segattach: a child writes to a shared segment, and the parent reads what it wrote. */
#include <u.h>
#include <libc.h>

void
main(void)
{
	long *p;
	Waitmsg *w;

	p = segattach(0, "shared", nil, 4096);
	if(p == (void*)-1)
		sysfatal("segattach: %r");
	p[0] = 1;
	switch(rfork(RFPROC|RFFDG)){
	case -1:
		sysfatal("rfork: %r");
	case 0:
		p[0] = 42;
		exits(nil);
	}
	w = wait();
	if(w == nil || w->msg[0] != 0)
		sysfatal("child: %s", w ? w->msg : "no wait");
	if(p[0] != 42)
		sysfatal("the parent reads %ld", p[0]);
	segdetach(p);
	exits(nil);
}
