/* semacquire and tsemacquire on a semaphore that two processes share through RFMEM. */
#include <u.h>
#include <libc.h>

long sem;
long done;

void
main(int argc, char **argv)
{
	int timed;

	timed = argc > 1 && strcmp(argv[1], "-t") == 0;
	if(timed){
		if(tsemacquire(&sem, 50) != 0)
			sysfatal("tsemacquire took a semaphore of 0");
	}
	switch(rfork(RFPROC|RFMEM|RFFDG)){
	case -1:
		sysfatal("rfork: %r");
	case 0:
		sleep(20);
		done = 1;
		semrelease(&sem, 1);
		_exits(nil);
	}
	if(timed){
		if(tsemacquire(&sem, 5000) != 1)
			sysfatal("tsemacquire: %r");
	}else if(semacquire(&sem, 1) != 1)
		sysfatal("semacquire: %r");
	if(done != 1)
		sysfatal("acquired before the release");
	if(waitpid() < 0)
		sysfatal("waitpid: %r");
	exits(nil);
}
