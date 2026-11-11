/* rendezvous: a parent and a child swap values on a tag. */
#include <u.h>
#include <libc.h>

void
main(void)
{
	void *v;

	switch(rfork(RFPROC|RFFDG)){
	case -1:
		sysfatal("rfork: %r");
	case 0:
		v = rendezvous((void*)0x9f, (void*)7);
		exits(v == (void*)11 ? nil : "child got the wrong value");
	}
	v = rendezvous((void*)0x9f, (void*)11);
	if(v != (void*)7)
		sysfatal("parent got %p", v);
	if(waitpid() < 0)
		sysfatal("waitpid: %r");
	exits(nil);
}
