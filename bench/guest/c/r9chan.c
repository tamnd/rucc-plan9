/*
 * r9chan: the channel benchmark of plan 13.2. Two threads of libthread send an int back and forth
 * on two channels with no buffer. It prints "bench chan NS OPS", as r9bench does.
 */
#include <u.h>
#include <libc.h>
#include <thread.h>

enum
{
	Ops = 200000,
};

static Channel *ping, *pong;

static void
echo(void*)
{
	long i;
	int v;

	for(i = 0; i < Ops; i++){
		v = recvul(ping);
		sendul(pong, v + 1);
	}
}

void
threadmain(int, char**)
{
	vlong t;
	long i;
	int v;

	ping = chancreate(sizeof(ulong), 0);
	pong = chancreate(sizeof(ulong), 0);
	threadcreate(echo, nil, 8192);
	v = 0;
	t = nsec();
	for(i = 0; i < Ops; i++){
		sendul(ping, v);
		v = recvul(pong);
	}
	t = nsec() - t;
	if(v != Ops)
		print("skip chan: the count is %d\n", v);
	else
		print("bench chan %lld %d\n", t, Ops);
	threadexitsall(nil);
}
