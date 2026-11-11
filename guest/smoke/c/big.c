/* A program with 2 MB of initialized data, which exec must load. */
#include <u.h>
#include <libc.h>

uchar big[2*1024*1024] = { 1 };

void
main(void)
{
	big[sizeof big - 1] = 2;
	if(big[0] != 1 || big[sizeof big - 1] != 2)
		sysfatal("bad data");
	print("big %d\n", (int)sizeof big);
	exits(nil);
}
