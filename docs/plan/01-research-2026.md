# Research, November 2026

This document records what was true when this folder was written: what 9front is, how big it is, what its compiler is, and what others have already built that runs Plan 9 programs on other systems. Every number was measured on the pinned tree (`2859ae04e`) on 9 November 2026, unless it says otherwise.

## 01.1 Which Plan 9

"Plan 9, latest, up to date" means 9front. Bell Labs' Plan 9 Fourth Edition stopped in 2015. The Plan 9 Foundation's 9legacy is a set of patches over it. 9front is the line that is maintained: it takes commits every week, makes releases, and has the drivers, the 64-bit ports and the GUI programs this folder needs.

| | |
|---|---|
| Canonical repository | `git://git.9front.org/plan9front/plan9front`. It could not be reached by HTTPS at that path on 9 November 2026. |
| Mirror used for the pin | `https://github.com/9front/9front`, branch `front` |
| Pin | `2859ae04e800737307470538a942ab8f8261253b`, 2026-11-07 22:41:41 +0000, "auth/factotum: kill ecdsa proto" |
| Checkout size | 287 MB |
| Current release | 11952, "THIS WAS SUPPOSED TO BE FUN", 2 August 2026 |
| Previous release | 11554, "GEFS SERVICE PACK 1", 24 January 2026 |
| Branches | one: `front`. There are no maintenance branches. A release is a dated snapshot with images, not a branch. |

Release 11952 brought these changes:
- the `affinewarp` drawing API;
- `gdbfs(4)`;
- Synaptics touchpads;
- `nusb/battery` and `nusb/dump`;
- SHA-3 in `libsec`;
- the `awk` test suite;
- x2APIC.

The release images, with their published SHA-256 sums:

| Image | SHA-256 (from 9front.org/iso) |
|---|---|
| `9front-11952.amd64.iso.gz` | `59ba79f1304f02c1aae573b337fa2569fc78027ce8e4c2056c4117e971025b91` |
| `9front-11952.amd64.qcow2.gz` | `3c1b57c4b609525e76cb0abac48bc657f71187fd3506674dba4e49b42141729c` |
| `9front-11952.arm64.qcow2.gz` (296,631,156 bytes) | `1199debe5e9cadadb9d143c9f64ef02c8a8010e173ebbff69aeaa23dff1a7457` |
| `9front-11952.386.iso.gz` | `b338c4cdf65d9fb8950b22dd21873d27d15254862cb9de8f7cd0c112b2663b05` |

There are also images for `pi`, `pi3`, `reform` and `honeycomb`. They are outside this folder's claim (02.6).

The release's source is the tree inside the image at `/sys/src`. `sys/lib/dist/mkfile` builds the images from a checkout, but no file in the image names the commit it was cut from. `r9 sets` therefore extracts the image's `/sys` tree and hashes it file by file. It walks `front` back from the release date until it finds the commit whose tree matches, and records that commit, or the exact list of differences if no commit matches (04.1).

## 01.2 How big it is

Counted with `wc -l` over the pinned tree:

| | Files | Lines |
|---|---|---|
| `.c` | 5,045 | 1,994,242 |
| `.h` | | 305,592 |
| `.s` | | 35,898 |
| C++ | 0 | 0. `sh.C` is the only `.C` file, and it is not C++. |

There is no C++ anywhere in the tree. Every compiled unit is C or Plan 9 assembly. So the BSD folder's C++ boundary (bsd 10) does not exist here, and rucc's share of the C units can be 100%.

By area:

| Area | C lines | Notes |
|---|---|---|
| `/sys/src/9` (all kernels) | 268,000 | plus 18,000 lines of assembly |
| of which `9/port` | 67,012, in 128 files | the portable kernel, which `9h` compiles unmodified |
| of which `9/pc` | 96,000 | PC drivers, shared by `pc` and `pc64` |
| of which `9/pc64` | 2,400 | the amd64-specific part |
| of which `9/arm64` | 3,000 | `qemu` virt and the generic arm64 code |
| `/sys/src/cmd` | 1,410,000 | every command, including the compilers, `rio`, `acme`, `sam` and `upas` |
| `/sys/src/ape` | 64,000 | the ANSI/POSIX environment |
| `/sys/src/games` | 90,000 | including the seven `eui` emulators and `doom` |
| `/sys/src/libc` | 23,000 | plus 7,500 lines of assembly |

## 01.3 The compilers: kencc

Ken Thompson wrote the Plan 9 C compilers. 9front maintains them in `/sys/src/cmd/cc` (the shared front end), `6c` and `7c` (back ends), `6l` and `7l` (linkers), and `6a` and `7a` (assemblers). `/$objtype/mkfile` names them:

| `objtype` | `CC` | `LD` | `AS` | `O` |
|---|---|---|---|---|
| `amd64` | `6c` | `6l` | `6a` | `6` |
| `arm64` | `7c` | `7l` | `7a` | `7` |
| `386` | `8c` | `8l` | `8a` | `8` |
| `arm` | `5c` | `5l` | `5a` | `5` |

`/sys/src/mkfile.proto` sets `CFLAGS=-FTVw` and `CPUS=spim arm arm64 amd64 386 power power64 mips`.

kencc is a single pass from source to a list of `Prog` records. It does no inlining across functions, a little register allocation, and almost no loop optimization. That makes it extremely fast and its code modest. The compiler is documented in `2c(1)`. The paper "How to Use the Plan 9 C Compiler" describes the dialect, and "A Manual for the Plan 9 assembler" the assembler. Document 06 takes the dialect from `cc/lex.c`, `cc/dcl.c` and `cc/macbody`, which are authoritative where they disagree with the paper.

GCC 16 has `-fplan9-extensions`, which accepts unnamed members with pointer promotion. It is the only Plan 9 feature GCC has. GCC cannot compile 9front: it has no `USED`, `SET` or `signof`, no `extern register` on the kencc registers, no Plan 9 ABI, and no `.6` output. It is still useful as a reference for the units it can parse (13.5).

## 01.4 The build and the tests

`/sys/src/mkfile` builds the whole system: `mk install` (or `mk release`, which is `nuke`, `install` and `clean`) for user space, and `mk kernels` for the kernels and the boot programs. Builds are in-tree:
- objects are written beside their sources (`foo.6`);
- installs go into the root (`/$objtype/lib`, `/$objtype/bin`).

Tests come from `/sys/src/cmd/mktest`. `mk test` runs them in 13 directories:
- `cmd/awk`, `cmd/cpp`, `cmd/diff`, `cmd/disk`, `cmd/git`, `cmd/test`;
- `libc`, `libdraw`, `libmach`, `libmemdraw`, `libmp`, `libregexp`, `libsec`.

There are 69 test programs and scripts in all. The `libc` tests are `atomic`, `date`, `lock`, `pow`, `runebreak`, `runenorm` and `strchr`. The suite is small. Most of the correctness signal in this folder therefore comes from the fixed point, from `diffrun` and from the GUI oracle (12), not from `mk test`.

## 01.5 The GUI programs

At the pin:
- about 109 C files call `initdraw` or include `eui.h`;
- 13 use `/dev/audio`.

The census of 02.7 groups them into 83 programs. Most are single files in `/sys/src/cmd`. The large ones are `rio`, `acme`, `sam`/`samterm`, `mothra`, `abaco`, `page`, `vnc`, `vt`, `audio/zuke`, `vmx`, `games/doom` and the `eui` emulators (2600, c64, gb, gba, md, nes, snes).

9front gives the GUI an oracle that no other system in this corpus has:
- **Input injection.** `kbdfs` serves `/dev/kbdin`, and the kernel serves `/dev/mousein`.
- **Output readback.** `rio` serves each window's image as `/dev/wsys/N/window`, and `devdraw` serves the screen as `/dev/screen`.
- **Determinism.** All drawing goes through `libmemdraw` in software.

Document 12.6 builds the oracle from these.

The arm64 `qemu` kernel configuration has no `draw`, `mouse` or `kbd` device (`9/arm64/qemu`). 9front on arm64 under QEMU is headless and is used through drawterm. That decides how arm64 GUI tests run (12.6).

## 01.6 Prior art: running Plan 9 programs elsewhere

| Project | What it does | What we take |
|---|---|---|
| drawterm (`github.com/9front/drawterm`, `be39bba`, 1 November 2026, MIT) | A host program containing a copy of part of the Plan 9 kernel in `kern/`: `chan`, `devmnt`, `sysfile`, `pgrp`, `devdraw`, `devcons`, `devip-posix`, `devfs-posix`, `devaudio`, `devkbd`, `devmouse`, `devpipe`, `devenv`, `devtls` and `qio`. It runs no Plan 9 binaries. It connects to a CPU server and exports the host's screen, keyboard, mouse, audio and files. GUI back ends are `gui-cocoa`, `gui-x11` and `gui-wl`. | The host back ends for screen, input, audio and files (10.6, 11). The proof that `port/devdraw.c` runs on a host. The negative lesson: drawterm's `kern/` is an edited copy that drifts, and `9h` must not be. |
| 9vx (Russ Cox, 2008; vx32) | A Plan 9 kernel as a host process, with user code sandboxed by vx32's x86 segmentation tricks. 386 only, unmaintained, and it does not work on amd64 hosts without segmentation. | The architecture of a kernel as a host process. The lesson that the sandbox must be a hardware mechanism that will last, which today means a hypervisor. |
| plan9port | Plan 9 libraries and programs ported to Unix as native host programs. They are built by the host compiler against a Unix libc, with many source changes, and `devdraw` is a separate host program. | Nothing for the "unmodified" claim. It is the reference for what host-native Plan 9 GUIs look like: fonts, HiDPI, snarf mapping (11). |
| Inferno `emu` | A hosted kernel running Dis bytecode, not native code. | Hosted devices, and the way host threads become kernel processes. |
| gVisor (KVM and systrap platforms) | A user-space kernel that runs Linux binaries. The KVM platform puts the application in guest ring 3, with a small guest ring 0. The systrap platform catches system calls with seccomp traps and a stub. | The design of `9h`'s executor (10.3) and of its Linux fallback without KVM (10.8). |
| Noah (2017) | Runs Linux binaries on macOS through Hypervisor.framework, with a host-side system call implementation. | The proof that HVF, with a guest trampoline that turns traps into exits, runs foreign binaries at native speed on macOS. |
| Apple Hypervisor.framework | On Apple Silicon: guest EL1 and EL0, stage-2 maps from host memory, `hv_vcpu_run`, and `hv_vcpus_exit` to kick a vCPU. Available since macOS 11, and on the M4 builder (`kern.hv_support=1`). | The executor on macOS arm64. Intel Macs have the x86 API with the same shape. |

No existing project runs unmodified 9front amd64 or arm64 binaries outside a full virtual machine. QEMU running 9front is the only way today, and it puts the whole of Plan 9, including its own window system, inside one emulated screen. `9h` is new. Document 10 shows it is built from parts that each exist.
