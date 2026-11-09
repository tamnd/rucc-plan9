# Performance

02.5 sets the gates:
- R's code runs at least as fast as G's;
- rucc compiles at least twice as fast as GCC 16 on the units both can compile;
- compile time against kencc is reported, and open question 1 decides whether it becomes a gate.

This document defines each measurement so that it can be repeated.

## 13.1 What "as fast" means for Plan 9

kencc does almost no optimization:
- it allocates registers within an expression;
- it folds constants;
- it does a peephole pass (`6c/peep.c`);
- it does nothing across statements.

So rucc at `-O2` should be faster on compute-bound code by a wide margin, and the gate (geometric mean ≤ 1.00) is a floor, not a target. The risks are elsewhere:
- **The convention.** Every register is caller-saved (08.2). A rucc that keeps values in registers across calls must spill and reload them around every call. Plan 9 code calls small functions constantly (`lock`, `qlock`, `print`, `Bgetc`). If rucc's allocator does not account for that, it can be slower than kencc's naive code.
- **Code size.** Inlining and alignment grow the text. The kernel and `libc.a` are gated at 1.25 × G (02.5). Larger code costs instruction-cache misses on the system call paths.
- **The kernel's hot paths.** These are system calls, `sched`, `qio`, `devmnt` and 9P. They are short and call-heavy, which is where the first risk bites.

The reference is G. There is no third-party compiler for Plan 9 to compare against, so G is also the only number a reader will ask for.

## 13.2 `r9 bench`

`r9-bench` is a Plan 9 C program in `rucc-plan9/bench/`, plus `rc` scripts that drive the tree's own tools. It is built by the build under test, so R's image runs R's `r9-bench`. The set is 40 benchmarks:

| Group | Benchmarks | Measures |
|---|---|---|
| System calls (5) | `errstr` round trip; 1-byte `read` and `write` on a pipe; `open` and `close` of `/dev/null`; `dirstat` of a cached path; `rendezvous` ping-pong between two processes | the system call path and `libc` stubs |
| Processes (4) | `rfork(RFPROC\|RFMEM)` with `wait`; `rfork` and `exec` of `/bin/true`; a context switch by pipe ping-pong; `semacquire`/`semrelease` between two processes | `sysrfork`, `sysexec`, segments, `sched` |
| Memory (4) | page faults over a 256 MB `segbrk`; `segattach` and `segdetach` of 1 MB; `malloc`/`free` mixes from 16 bytes to 1 MB (`pool`); `libthread` channel ping-pong | `fault`, `segment`, `pool`, `libthread` |
| Files and 9P (5) | write and read 100 MB through `ramfs`; extract a fixed 100 MB tar of small files onto gefs; `du -a` over the result; `exportfs` over a pipe, reading 100 MB; `mk -n` in `/sys/src/cmd` | `devmnt`, the 9P path, `qio`, gefs |
| Network (2) | TCP over loopback, one stream and 1 GB; UDP 64-byte packets per second | `devip`, the TCP stack, `qio` |
| Tools (8) | `rc` running a fixed loop and expansion script; `awk` and `sed` over a 200 MB log; `grep` with a fixed string and a regular expression; `sort` of 2 million lines; `gzip -9`; `bzip2`; `troff` of the whole manual | user-space C |
| Libraries (8) | `qsort`; `strtod`; `print("%g")`; `regcomp` and `regexec` (`libregexp`); `memmove` at 12 sizes; RSA-2048 signing (`libsec`, `libmp`); SHA-256 of 1 GB; AES-CBC of 1 GB | `libc`, `libregexp`, `libmp`, `libsec` |
| Graphics (4) | 100,000 `draw` calls of mixed sizes and operators through `libmemdraw` in user space; the same through `/dev/draw` to the kernel's `devdraw`; `acme` scrolling a 50,000-line file by script; `doom -timedemo demo1` frames per second | `libmemdraw`, `devdraw`, `rio`, `acme` |

The set is fixed per major release of `r9-bench`, so geometric means stay comparable over time. Adding a benchmark is a release.

**Text size.** Also reported:
- the text size of the `9pc64` kernel, from `size`;
- the total text of the members of `/amd64/lib/libc.a`;
- the total text of `/amd64/bin`.

The kernel and `libc.a` are gated at 1.25 × G (02.5).

## 13.3 Method

**The machine.** server3:
- SMT off;
- the CPU frequency fixed;
- no other guest running.

The guest runs under QEMU/KVM, pinned to four host cores with 4 GB. The R and G images differ only in the build that made them.

**The runs.**
1. Interleave R and G: boot one, run the set once, shut down; then boot the other. Repeat 10 times, for 20 boots.
2. For each benchmark, the gated number is the R/G ratio of the medians over 10 runs per side.
3. A benchmark whose coefficient of variation exceeds 1% on G is reported but not gated, until it is fixed or removed. Graphics benchmarks run with `rio` and nothing else on screen.
4. The headline is the geometric mean of the gated ratios.

Kernel and user space are measured together, as the user will run them. The 2x2 (12.7) splits any regression: `r9 bench --kernel=G --user=R` and its mirror take one more run of the set.

**Cadence.** The set runs weekly. It also runs on any rucc commit that changes amd64 code generation by more than 0.5% in the text size of `libc.a` or of the kernel. A full run takes about 2 hours (04.5).

**When a benchmark is slow.**
1. `r9 mix bisect --perf` uses the bisection of 12.8, timing the benchmark instead of passing a test. It finds the unit whose R object costs the time.
2. Then 9front's kernel profiler (`/dev/kpdata`, the `kprof` device) or user profiling (`prof` with `-p`) on both images compares the hottest functions.

The result is a `rucc-corpus` performance case (15.3).

## 13.4 Graphics on H

`9h` adds a question 13.2 does not answer: does a program run as fast on H as on Q? This is reported, not gated, because H's speed is the monitor's and the host's, not rucc's.

| Number | What |
|---|---|
| `doom -timedemo` frames per second | R/H against R/Q, on the same CPU architecture |
| `acme` scroll time | R/H against R/Q |
| `flushmemscreen` hypercalls per second, and presentation frames per second | from the monitor's counters, at the end of each GUI script |
| first `rio` frame after `9h run` | wall time (10.6) |

amd64 R/H runs on gpc under KVM, and amd64 R/Q runs on the same host for the comparison. arm64 R/H runs on the M4 under HVF, against arm64 R/Q under QEMU/HVF on the same M4.

## 13.5 `r9 speed`: compile time

**Sources.** `r9 ccd` records, for each compile:
- rucc's user and system CPU time;
- its peak resident memory;
- the time spent waiting on 9P, separately.

For G, `r9-cc` in recording mode times the real `6c` with `/proc/n/status`'s CPU times. Both sides count only the compiler process: for G, `6c` alone, and for R, rucc alone. The shim and its network round trips are on both sides and are excluded.

**Against GCC 16 (the gate).** The parent goal is twice as fast as GCC 16. GCC cannot compile Plan 9 code for Plan 9, but GCC 16 with `-fplan9-extensions` compiles much of the tree for a host target: 685 of 962 units in the 03.18 sample. The comparison runs on server3's host, outside the guest:
1. **The units.** `r9 speed --gcc` takes every unit that GCC 16 compiles with `-fplan9-extensions -std=gnu99 -O2 -c` for `x86_64-linux-gnu`. The headers and sources come from the pin's checkout, and the macros are the ones each unit's mkfile gives.
2. **The comparison.** rucc compiles the same units, for the same target, with the same flags (`-fplan9-extensions`, 06.10).
3. **The gate.** The sum of rucc's CPU time over the units, divided by GCC's, must be at most 0.5. `r9 speed` also reports the 50th, 90th and 99th unit percentiles, and the 50 worst units with their preprocessed sizes.

The objects are ELF and never run. This measures compile speed on Plan 9's code, which is what the parent goal asks, and it uses no Plan 9 target that GCC lacks.

**Against kencc (reported).** rucc in the R build, targeting `x86_64-unknown-plan9` at `-O2`, against `6c`, per unit and in total. P0 measures `6c`. Expect it to be very fast: a single pass, no SSA, and no optimizer, at about 5 ms for a typical unit (05.4). For rucc to be twice as fast, it would need about 2.5 ms per unit, including its optimizer. That is why it is open question 1 and not a gate. Also reported:
- rucc at `-O0` against `6c`, which is the fairer comparison of front ends;
- the share of rucc's time in the front end, the optimizer, and the `.6` writer.

**Build wall time.** R's whole build against G's, reported with its parts: rucc CPU, 9P wait, shim and network round trips, and `mk`. The 9P wait is the part `rucc-9p`'s cache must keep small (05.4). A wall time over three times G's is a performance issue in `r9`, filed against `rucc-plan9`, not rucc.

**Memory.** No rucc process may exceed 1 GB (02.5). The largest unit at the pin is reported with its peak.

## 13.6 What is not measured

- **386 and arm.** They have no gates until P10 and P11 add them (16).
- **arm64 run-time speed against G.** It is measured on the M4 under HVF, but the M4 is also a development machine, so its numbers are informational.
- **Programs outside the tree.** There are none to measure. 9front has no ports collection.
- **Boot time.** It is reported in the boot test (12.3), not gated. It is dominated by the disk and the timeout settings.
