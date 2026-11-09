# The build: `mk`, `objtype`, and putting rucc in the compiler slot

Plan 9 builds itself with `mk` and `rc` in a Plan 9 namespace. Nothing about that can be reproduced faithfully on a Unix host: plan9port's `mk` and `rc` exist, but `/$objtype`, union directories and `bind` do not. So the build runs in a 9front guest. This document explains how rucc, a host program, becomes that guest's `6c` without changing a file in the tree.

## 05.1 How the tree reaches the compiler

Every compile in the tree goes through one of three paths. P0's census confirms that there is no fourth.

**1. `$CC` in a mkfile.** `/$objtype/mkfile` sets `CC=6c`. The generic rules in `/sys/src/cmd/mkone`, `mkmany`, `mklib` and `mksyslib` compile with `$CC $CFLAGS $stem.c`, which writes `$stem.$O` in the current directory. Per-directory mkfiles add flags. These are the most common forms at the pin:

| Form | Count |
|---|---|
| `CFLAGS=$CFLAGS -I../common` | 11 |
| `CFLAGS=-I../lib $CFLAGS` | 11 |
| `CFLAGS=-c -D$SYSTEM -D_POSIX_SOURCE -I$COMMONDIR -B` | 10 |
| `CFLAGS=-c -D_POSIX_SOURCE -D_PLAN9_SOURCE` (APE) | 9 |
| `CFLAGS=$CFLAGS -p ...` (full `cpp`) | 11 directories |
| `CFLAGS= -I. -I../port $CFLAGS` (kernels) | 2 |

**2. `pcc`, APE's driver.** 26 mkfiles set `CC=pcc` and 15 set `LD=pcc`. `pcc` (`cmd/pcc.c`) runs `/bin/cpp` with `-D__STDC__=1 -N` and APE's include paths, and pipes the output into `/bin/$cc -o file.$O`, which reads the source on standard input. So the compiler slot also receives preprocessed C on stdin, with `#line` directives.

**3. Kernels.** `/sys/src/9/$arch/mkfile` compiles port and arch files with `$CC $CFLAGS`. It builds the configuration's generated `$CONF.c` with the same rule (09.2).

Two programs bypass `mk` and call a compiler by name:
- `mk`'s `NPROC` parallelism, which still runs `$CC`;
- `cmd/cc`'s `mkfile` for the compilers themselves.

The `r9-cc` shim catches both, because it sits where the name resolves.

## 05.2 The build image

A build runs in a throwaway guest disk made by `r9 image build`:
1. Start from the release image of 04.1, which has the bootstrap tools.
2. Replace `/sys/src`, `/sys/include`, `/sys/lib`, `/sys/man`, `/lib`, `/rc` and `/$objtype/include` with the pinned tree's. 9front's own `sysupdate` does this from git. `r9` writes the files from the checked-out tree, through a 9P copy (`r9 image sync`), so the guest needs no network.
3. Build G's tools (04.2) and install them.
4. Save the result as `g-base.qcow2`, one per pin and `objtype`, with its hash in `baselines/`.

R and G builds both start from a copy-on-write overlay of `g-base`. They run `mk install` and `mk kernels`, and the overlay is thrown away after its outputs are collected. Collecting means taking:
- the installed tree;
- the `.6`/`.7` files, kept beside their sources;
- the logs.

The root file system is gefs, as in 11952's default install. The disk is a host file. That matters on macOS, where the host file system is case-insensitive and 25 paths in the tree, in 12 groups, differ only by case (11.8). Nothing in this design puts the tree on a host directory.

## 05.3 The compiler slot: `r9-cc`

`r9-cc` is a Plan 9 C program in `rucc-plan9/guest/r9-cc`, about 600 lines. It is built by G's `6c`/`7c` into the base image. The build's namespace binds it over the compilers. The binding is done in the `rc` script `r9-build`, which is the guest's whole build command:

```rc
#!/bin/rc
# r9-build: the only thing r9 runs in the guest. It changes the namespace, never a file.
rfork ne
bind /usr/glenda/r9/bin/r9-cc /$objtype/bin/6c     # amd64
bind /usr/glenda/r9/bin/r9-cc /$objtype/bin/7c     # arm64, when objtype=arm64
for(c in 8c 5c vc) bind /usr/glenda/r9/bin/r9-cc /$objtype/bin/$c   # recorded as other-arch
cd /sys/src && mk install >[2=1] && mk kernels >[2=1]
```

`/bin` is a union that includes `/$objtype/bin`, so `/bin/6c`, which `pcc` uses, also resolves to the shim. G builds bind the same shim in recording mode. That puts the two builds on the same path, and the shim's overhead out of the comparison.

`r9-cc` does four things for each call:
1. **Identify the call.** It reads its own `argv[0]` to tell `6c` from `7c` (and later `8c` and `5c`). It reads `objtype`, the current directory, the arguments, and whether the source is on stdin.
2. **Export the namespace.** On the first call in a build, it dials `r9 ccd` at `tcp!10.0.2.2!<port>` (QEMU's user network routes that address to the host). It then serves `exportfs` of the build's namespace on a second connection it dials itself, the way `cpu(1)` exports a terminal's namespace. That namespace is the one of the shim's own process group. The host is the 9P client, and the guest needs no listening port. Every later call reuses that export, through a small `/srv` entry the first call posts. The export serves what the compiler would see: the binds of `r9-build`, `/sys/include`, `/$objtype/include`, and the source directory.
3. **Ask the host.** It sends a request to `r9 ccd`. The request is a small text message: the version, the `cc` name, `objtype`, the directory, `argv`, and the stdin bytes if any. The reply carries:
   - the exit status;
   - standard error, written byte for byte;
   - each output file's contents and name.
4. **Write the outputs** in the guest, at the path kencc would have used (`-o`, or `stem.$O` in the current directory). The guest file system holds the objects, as it would with kencc.

In recording mode, `r9-cc` also runs the real compiler. It saves the true `6c` at bind time with `bind /$objtype/bin/6c /n/r9/real6c`, before the shim is bound over it. It appends a record to `/tmp/r9/compile.jsonl`, the same record format as `harness-shim` (15.4). The record has:
- `argv`;
- the directory;
- the input hash;
- the output hash;
- the time;
- the origin: `rucc`; `ref-c`; `ref-as` for `6a` calls, which `r9-as` records the same way from P6; or `other-arch` for a compiler outside the claim (`8c`, `5c`, `vc`; 09.1).

## 05.4 The host side: `r9 ccd` and rucc

`r9 ccd` is the host daemon, in the `r9` binary. It accepts requests and runs rucc with the arguments translated:

```
rucc --driver=kencc --target=x86_64-unknown-plan9 \
     --sysroot=9p:fd!<session> --cwd=/sys/src/cmd/acme \
     -FTVw -I../common -o acme.6 acme.c
```

`--driver=kencc` makes rucc's driver speak kencc's command line exactly (06.11). The flags pass through unchanged. rucc reads every file through the 9P sysroot of 03.7. When the request is from `pcc`, the source is the stdin bytes, and the file name for diagnostics comes from the first `#line`.

rucc writes its outputs to memory, and `r9 ccd` returns them. rucc never writes into the guest directly. That keeps the guest file system the only place the build's state lives, and makes the build restartable at any `mk` step.

**The 9P cost.** A typical 9front compile includes `u.h`, `libc.h` and 3 to 10 more headers. Over 9P that is about 30 to 80 round trips (`walk`, `open`, `read`, `clunk`) on a local QEMU network. At 50 to 100 µs each, that adds 2 to 8 ms to a compile kencc does in about 5 ms. The 9P client caches by `qid.path` and `qid.vers` for the length of a build session, so headers are fetched once per build. Measured compile time is rucc's process CPU time, and 9P wait is reported separately (13.5).

**Linux builders** can mount the export with the kernel's v9fs (`mount -t 9p -o trans=tcp,port=…,version=9p2000`). rucc then reads a host path as an ordinary sysroot. P0 uses that mount, before `rucc-9p` exists. **macOS** has no 9P mount in the kernel, so the M4 builder needs `rucc-9p` from P1.

## 05.5 Identity

kencc has no identity a build can ask about. There is no `--version`, no banner, no `__GNUC__`, and no predefined macros. The tree's mkfiles never probe the compiler. They name it by `objtype`. "Configured the same" is therefore simpler here than in bsd 05: it means the same argv for every unit, which `r9 flags-diff` checks over the two `compile.jsonl` files.

One thing must still match: rucc, as `6c`, must produce exactly the files kencc produces, and no others:
- `stem.6`;
- `-a`/`-aa` acid files;
- `-n` acid output into `stem.acid`.

`r9 tree-diff` compares the full set of build outputs, the installed tree and all the intermediate files, so an extra or missing file is a failure.

## 05.6 The census

`r9 census` reads `compile.jsonl` from an R build and states:
- the number of `.c` compiles, with their origin;
- the number of `.s` assemblies, with their origin;
- the units that went through `pcc`;
- the units compiled more than once (the same file in different directories), and whether their outputs agree;
- what never reached the compiler slot: any `.6` in the installed tree whose producer has no record;
- the `other-arch` units, by `objtype`.

For a graded run, `ref-c` must be 0, and the "never reached the compiler slot" item must be empty. `other-arch` is reported, not graded. Until P6, `.s` files are `ref-as`. From P6, they are `rucc` too.

`r9 demands` runs at P0 over G's records. It replays each unit's argv through `rucc --driver=kencc -fsyntax-only` and ranks the first error per unit, the way `rb demands` did. That ranking orders P1's work.

## 05.7 Mixed objects

kencc objects and rucc objects must link together in any proportion, because:
- **Mixed-object bisection** (12.8) builds a program from R and G objects and searches for the culprit unit.
- **The 2x2** (12.7) mixes R and G at the kernel and user-space boundary.
- **Bring-up** delegates units.

The conditions are:
- **The calling convention.** rucc uses the same one (08.2).
- **The symbol names.** kencc uses no prefix and no decoration, apart from static symbols with `<>`.
- **The type signatures.** These are the same hash (07.4).
- **The object records.** They come in the version `6l` reads (07.1).

`r9 mix --file=<unit>=R|G` builds any mix. `6l` rejecting a mix because of a signature difference is a rucc bug (03.13).

## 05.8 Warnings

The tree builds with `-w`. kencc's warnings never fail a build, so warning parity is not a correctness property here, as it was in bsd 06.9. `r9 warn-diff` reports, per unit:
- the warnings G gives and R does not, which is lost checking;
- the warnings R gives and G does not, which is noise.

The report is published but does not grade. The `varargck` format warnings are the one class P3 drives to parity, because they are the main thing `-F` buys (03.12).
