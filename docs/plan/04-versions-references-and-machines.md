# Versions, references and machines

## 04.1 Sets and pins

`rucc-plan9/pins.toml` records every pinned input with its hash. `r9 sets --write` regenerates it from what 9front publishes, and a hand edit fails `r9 check`.

```toml
[tree.front]
repo   = "https://github.com/9front/9front"   # mirror of git.9front.org/plan9front/plan9front
branch = "front"
commit = "2859ae04e800737307470538a942ab8f8261253b"
date   = "2026-11-07T22:41:41Z"
policy = "weekly: newest commit G builds and passes (04.2)"

[tree."11952"]
image  = "https://9front.org/iso/9front-11952.amd64.qcow2.gz"
sha256 = "3c1b57c4b609525e76cb0abac48bc657f71187fd3506674dba4e49b42141729c"
arm64  = { image = "https://9front.org/iso/9front-11952.arm64.qcow2.gz", sha256 = "1199debe5e9cadadb9d143c9f64ef02c8a8010e173ebbff69aeaa23dff1a7457" }
commit = "<found by r9 sets (01.1)>"

[tree."11554"]
image  = "https://9front.org/iso/9front-11554.amd64.qcow2.gz"
sha256 = "<from 9front.org/iso, checked at P0>"

[drawterm]
repo   = "https://github.com/9front/drawterm"
commit = "be39bba"          # 2026-11-01, kern/devdraw.c synced with 9front
use    = "reference for 9h's host back ends; not built into graded runs"

[qemu]
version = "11.1.2"
```

A `front` pin advances weekly. The candidate is the newest commit for which G builds, boots and passes its baseline three times (12.1). If G fails, the pin stays where it is and the report says why. 9front's `front` sometimes breaks briefly after large changes. How often, and for how long, is unknown until P0 has built a year of weekly commits with G. Open question 7 asks whether a week is too slow.

## 04.2 The reference, G

G is kencc from the same pin, built by itself. It is made in three steps:
1. Boot the release image of the current release in QEMU. It provides the bootstrap `6c`, `6l`, `6a`, `mk` and `rc`.
2. Put the pinned tree in place of the image's `/sys/src` (05.2). Build and install the compilers with the bootstrap tools: `cd /sys/src/cmd && mk 6c.install 6l.install 6a.install 7c.install 7l.install 7a.install`. Then build them again with themselves.
3. Check that the second build's binaries are byte-identical to the first's. That is kencc's own fixed point. It is expected to hold because kencc writes no time stamps or host paths, and P0 checks it at the first pin. Those binaries are G's tools.

G's build of the tree is `mk install` and `mk kernels` with G's tools, run three times. The run records:
- the per-unit commands and `.6` outputs, from the shim in recording mode;
- the installed tree;
- the test results.

**Why not GCC.** No GCC has built 9front, and none can (01.3). A GCC reference would have to be a different program, built from a different source. The parent goal's "GCC 16 compatible" is still enforced on every rucc commit by `rucc-compat`. Here GCC 16 is a second reference for exactly two things:
- whether a unit parses (13.5);
- `-fplan9-extensions`' semantics (06.10).

**Why not the release binaries.** 9front's release images are built by the 9front maintainers from a commit near the release date, with that commit's kencc. Our G is built on our rig from our pin, so differences in the time stamp, the host or the tool binaries cannot creep in. The release image's binaries are a cross-check: `r9 xcheck` compares G's installed tree with the image's and reports differences without grading them.

**The persona.** There isn't one. kencc has no version banner, no `__GNUC__` and no `-v` that a build parses. When `r9-cc` hands a unit to rucc, rucc is invoked as `6c` or `7c` and answers to nothing else (05.3).

## 04.3 What G gives that GCC did not

| | BSD folder (GCC) | This folder (kencc) |
|---|---|---|
| Determinism | needs `-frandom-seed`, `SOURCE_DATE_EPOCH`, path prefixes | kencc is deterministic by design: no time stamps, no absolute paths beyond `#line` history |
| Object diff | ELF section diff with debug info stripped | `.6` record diff: per-symbol `Prog` lists, readable (07.6) |
| Self-hosting | GCC is not built by the base | kencc is part of the tree, so the fixed point is a test (12.4) |
| Speed | `-O2` GCC is slow, so 2x is plausible | kencc is very fast, so 2x is doubtful (13.5, question 1) |

## 04.4 Machines

Linux builders are those of `test-machines.md`:

| Machine | Cores | Role here |
|---|---|---|
| gpc | 32 x86-64, KVM | G and R builds for amd64 and arm64 (arm64 on TCG); nightly; Linux amd64 `9h` GUI runs under Xvfb and a Wayland headless compositor |
| server3 | 8 x86-64, KVM | the performance box: runtime benchmarks of 13.1, pinned guest, no other jobs |
| server1, server2 | 4 and 6 | `diffrun` and the `mk test` fan-out; weekly release runs |
| the M4 Mac | Apple M4 (Mac16,13), 10 cores, 24 GB, macOS 15.8.2, `kern.hv_support=1`, QEMU 11.1.2, GCC 16.2.0 | arm64 Q under QEMU/HVF (fast arm64 builds and tests); macOS `9h` GUI runs; `r9 hostcheck` on Cocoa |

Two more machines are wanted and do not exist:
- **An arm64 Linux machine with KVM**, to run `9h` on Linux arm64 and to give arm64 Q a native speed on Linux. An Ampere host would do. A Raspberry Pi 5 probably would not, because its interrupt controller is GICv2 (open question 6).
- **An Intel Mac.** It is wanted only if amd64 `9h` on macOS is to be claimed. It is not in the claim (02.7).

QEMU is pinned at 11.1.2 on every builder, built from source with the same configure line, so Q is the same virtual machine everywhere. The amd64 Q machine is `-machine q35 -cpu host -smp 4 -m 4G -vga std -device virtio-net-pci -drive if=virtio`. The arm64 Q machine is `-machine virt,gic-version=3 -cpu host` (HVF) or `-cpu max` (TCG) with `-smp 4 -m 4G`, headless as 9front's `qemu` kernel requires.

## 04.5 How long things take

These are estimates. P0 replaces them with measurements:

| Job | Where | Estimate |
|---|---|---|
| G full build (`mk install` and `mk kernels`), amd64 | gpc guest, 4 vCPU | 6 to 10 minutes. kencc is fast; `mk` runs one job per directory unless `NPROC` is set, and the tree's mkfiles honour `NPROC` |
| R full build, amd64, through `r9-cc` | gpc guest and host | 20 to 40 minutes before caching; the 9P round trips dominate (05.4) |
| `mk test`, amd64 | gpc guest | under 5 minutes |
| `diffrun` | server1/server2 | 15 minutes |
| GUI suite, one host, 80 scripts | gpc or the M4 | 30 to 45 minutes |
| Performance suite | server3 | 2 hours |

A nightly cycle covers:
- G build (cached per pin);
- R build;
- boot;
- `mk test`;
- `diffrun`;
- the GUI suite on two hosts.

It fits in three hours.
