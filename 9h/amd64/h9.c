/*
 * The 9h machine (rucc-plan9, plan 10.2): the functions that
 * pc64 main.c and pc/bootargs.c call in the screen files,
 * which the 9h kernel does not have. mkdevc writes an empty
 * bootscreeninit, because the configuration has no vga, so
 * this file does not have it. The screen of the machine
 * is in h9screen.c, which does not use the vga files.
 */
#include "u.h"
#include "../port/lib.h"
#include "mem.h"
#include "dat.h"
#include "fns.h"
#include "io.h"

/* screen.c */
char*
rgbmask2chan(char *buf, int, u32int, u32int, u32int)
{
	buf[0] = 0;
	return buf;
}

/* vgavesa.c */
char*
vesabootscreenconf(char *s, char*, uchar*)
{
	return s;
}
