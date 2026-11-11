/* notes and alarm: a note to the process itself, and an alarm that ends a sleep. */
#include <u.h>
#include <libc.h>

int got;

void
handler(void*, char *msg)
{
	if(strcmp(msg, "r9") == 0 || strcmp(msg, "alarm") == 0){
		got++;
		noted(NCONT);
	}
	noted(NDFLT);
}

void
main(int argc, char **argv)
{
	long t;

	notify(handler);
	if(argc > 1 && strcmp(argv[1], "-a") == 0){
		t = time(0);
		alarm(200);
		sleep(10000);
		if(got != 1)
			sysfatal("no alarm note");
		if(time(0) - t > 5)
			sysfatal("the alarm did not end the sleep");
		exits(nil);
	}
	if(postnote(PNPROC, getpid(), "r9") < 0)
		sysfatal("postnote: %r");
	sleep(100);
	if(got != 1)
		sysfatal("the note did not come");
	exits(nil);
}
