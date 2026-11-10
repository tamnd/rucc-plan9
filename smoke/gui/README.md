# The GUI smoke test

This test runs GUI programs that rucc compiled in a 9front guest, and keeps a picture of the screen. It is a first step to P8. It does not use the host back ends of document 11. The draw device comes from `vncs`, which needs no display and no network outside the guest.

## What it builds

`gather.py` compiles these files with the kencc driver of rucc and the CFLAGS of each mkfile:

- The 17 libraries that the programs use: libc, libdraw, libmemdraw, libmemlayer, libframe, libthread, libplumb, libcomplete, libbio, libregexp, libsec, libmp, libauth, libauthsrv, libip, libcontrol and libflate. The names come from OFILES, CFILES or FILES of each mkfile. A port file that the directory of the machine replaces is not compiled, as the `reduce` scripts of the tree do.
- rio, acme, samterm and vncs.
- clock, colors, stats, winwatch, paint and catclock.

`libauthsrv/authpak.c` needs `msqrt.mpc`, which the build makes with `mpc`. The script does not compile it, and the guest keeps the object of the image.

## What the guest does

`run.rc` puts the objects of rucc into a copy of each library of the image and binds the copies over `/$objtype/lib`. It links each program with the loader of the image, and binds the programs over `/bin`. Then `vncs` runs `gui.rc`, which starts rio with the windows of `start`. After 25 seconds, `start` copies `/dev/screen` to a file. `run.rc` converts it to PNG and writes it to the collect disk with the standard error of each window.

## How to run it

```sh
python3 -I smoke/gui/gather.py PATH/TO/rucc ~/.cache/r9/trees/front arm64 /tmp/gui/in
r9 boot --row arm64 --accel hvf --run smoke/gui/run.rc --in /tmp/gui/in --out /tmp/gui/out --collect 64
```

The picture is in `/tmp/gui/out/collected/shot.png`. To make the picture of G, remove the line `bind -b /tmp/r/bin /bin` from a copy of `run.rc` and run it with the same input.

## The result

The sources are the `front` tree at 2859ae04e800, and the image is release 11554. The compiler is rucc with the changes of tamnd/rucc#4312 and the pull requests under it. On arm64 under HVF on an M4, all 604 files compile and all 10 programs link. On amd64 under TCG on the same host, all 590 files compile and all 10 programs link. The picture of R is the same as the picture of G, except in these areas:

- The hands of clock and catclock, and the graph of stats, which change with time.
- catclock, which is in `/bin/games` and not in `/bin` on the image. The G picture has no catclock window, and winwatch shows one name less.
