# Spec 2131/corpus/plan9: 9front, unmodified, built by rucc, and its GUI on macOS and Linux

rucc builds an unmodified 9front, the maintained Plan 9: the kernels, libc and every library, the compilers and linkers themselves, every command in `/sys/src/cmd`, the games and the boot code. It does this by being a drop-in replacement for the Plan 9 C compilers `6c` and `7c`, so the tree's own `mk` files call it without knowing. The system it builds boots, passes the tree's own tests, and produces byte-identical output wherever the program is deterministic. The same tree also runs on macOS and Linux. A new program, `9h`, compiles 9front's portable kernel, `/sys/src/9/port`, unchanged with rucc for the host. It runs unmodified Plan 9 binaries in user mode on the host CPU through Hypervisor.framework or KVM, and puts every graphical program on the host desktop: `rio`, `acme`, `sam`, `page`, `mothra`, the emulators, `doom` and the rest. Every compiler change lands in `tamnd/rucc`. The harness and `9h` live in a new repository, `tamnd/rucc-plan9`. The existing repositories grow to carry what Plan 9 teaches: `tamnd/rucc-compat`, `tamnd/rucc-corpus` and `tamnd/rucc-harness`.

Written 9 November 2026, against rucc 0.30.15 (`tamnd/rucc` main, commit `5f9810aab`). The pin is 9front `front` at `2859ae04e` (7 November 2026, "auth/factotum: kill ecdsa proto"), from the GitHub mirror `github.com/9front/9front`, with releases 11952 ("THIS WAS SUPPOSED TO BE FUN", 2 August 2026) and 11554 ("GEFS SERVICE PACK 1", 24 January 2026). This folder sits beside `corpus/kernel` and `corpus/bsd`. It follows their shape, their rules and most of their machinery. Where it disagrees with them, it is because Plan 9 is a different kind of system, and the document says why.

## Why Plan 9 is different from the BSDs

The BSD folder made rucc a compiler for a whole GCC-built operating system. Plan 9 asks for five things the BSDs did not.

**The reference compiler is not GCC.** Plan 9 is built by its own compilers: `6c` for amd64, `7c` for arm64, `8c` for 386, `5c` for arm. Together they are called kencc. No GCC has ever built 9front, and none can: the tree uses a C dialect that GCC only partly accepts (06). So G here is kencc, not GCC 16. The parent goal, "100% compatible with GCC 16", still holds for rucc as a whole. In this folder it means something narrower: everything rucc adds for Plan 9 sits behind a target or a flag, and nothing changes rucc's behaviour for any GCC target (02.9).

**It has its own C dialect.** The dialect has:
- unnamed struct members that are promoted and convert implicitly to pointers;
- `USED`, `SET`, `signof` and `typestr`;
- `#pragma varargck`, which drives format checking;
- `extern register` globals that hold the current processor and process;
- a preprocessor that has no `#if` unless asked for one, and that substitutes macro arguments inside strings.

Document 06 lists all of it, with counts from the tree.

**It has its own ABI, data model, object format and executable format.** `long` is 32 bits on every architecture. Arguments are passed on the stack, apart from the first, which goes in one register. No register is callee-saved. Objects are not ELF: they are serialized instruction streams that only `6l` and `7l` can read. Executables are Plan 9 `a.out`. rucc has none of this today (03, 07, 08).

**The build runs on Plan 9, not on the host.** `mk`, `rc`, per-process namespaces and `bind` are how the tree builds. rucc is a Rust program, and Rust has no Plan 9 target. So the build runs in a 9front guest, and its compiler slot is a shim that forwards each compile to rucc on the host. rucc reads the guest's namespace over 9P (05).

**"Run the GUI apps on macOS and Linux" is a claim about an operating system, not a compiler.** A Plan 9 GUI program speaks to the kernel through files: `/dev/draw`, `/dev/mouse`, `/dev/cons`, `/dev/snarf`, `/dev/wsys`. Running it unmodified means providing that kernel. `9h` does so by compiling 9front's own portable kernel with rucc, against a host "architecture" directory that lives in `rucc-plan9`. That directory provides devices such as `/dev/draw` and audio through Cocoa or X11/Wayland, CoreAudio or PipeWire, host sockets and host files (10, 11). The same design is what makes `9h` a strong compiler test: 67,000 lines of kernel C, compiled by rucc, running real programs.

## Where we start from

Document 03 is the full inventory. The short version:

**What already works.** About 70% of the C in the libraries and in the GUI programs already parses, with three stand-ins: `-fms-extensions`, `-std=gnu89`, and `USED` and `SET` defined as macros. On 962 files from libc, the graphics libraries, `acme`, `rio`, `sam`, `rc`, `mk`, `9/port` and the compilers, 687 pass `-fsyntax-only`. GCC 16 with `-fplan9-extensions` passes 685 of them. rucc's front end, optimizer, x86-64 and AArch64 back ends, the integrated assembler and the kernel code model (from `rucc-kernel`) are all in place.

**What is missing or wrong.**
- **There is no Plan 9 target** (03.1). `x86_64-plan9` and `amd64-unknown-plan9` are refused with "unknown operating system".
- **The dialect is missing** (03.2):
  - pointer promotion of unnamed members;
  - `.Type` member access;
  - `USED`, `SET`, `signof` and `typestr`;
  - the pragmas, which are silently ignored;
  - macro arguments inside strings;
  - `extern register` (error E0714, on 54 of the 962 files);
  - `-fplan9-extensions`, which GCC 16 has and rucc does not.
- **Promotion is silently wrong in one mode.** Under `-std=gnu89`, the pointer promotion of an unnamed member is accepted with only a warning, and the offset is not applied (03.2).
- **There is one real C bug.** rucc refuses brace elision over an array of pointers initialised from strings, which GCC accepts (03.3).
- **The kencc ABI, the `.6`/`.7` object writers and the `a.out` format are missing** (03.4 to 03.6).
- **There is no 9P client in the driver** (03.7).

About 28 to 38 engineer-weeks of rucc work is visible now. That is more than the BSD folder's estimate, because Plan 9 needs a new ABI and two new object formats. The unknown is the code generation bugs that 2.3 million lines of new C will find.

The whole plan, including the harness, `9h`, the GUI scripts and 386 and arm, is 82 engineer-weeks, or about 91 with contingency (16.14). With two engineers it runs from November 2026 to about December 2027. The smallest claims that stand alone are amd64 9front built by rucc (34 weeks) and the GUI on macOS and Linux with G's build (18 weeks, no rucc work).

## The goal, stated so it can be falsified

Document 02 states each claim precisely. In short:

**Unmodified.** The source is the pinned 9front commit or a release's own tree, checked by hash. Nothing in it is patched, added or regenerated. Configuration is the kernel configuration files the tree ships, the `objtype` environment variable, and a namespace in which `/$objtype/bin/6c` (or `7c`) is the `r9-cc` shim. That last one is a `bind`, not an edit. Everything else applies to G as well.

**Built.** In the guest, `cd /sys/src && mk install` and `mk kernels` both succeed with rucc in the compiler slot. Every C unit in the tree is compiled by rucc, and the census proves it (05.6).

**Correct.** These checks, set out in 12:
- the 9front image built by R boots under QEMU;
- `mk test` passes every test G passes;
- the compiler fixed point holds: `6c` built by R produces byte-identical `.6` files to `6c` built by G;
- 40 deterministic programs produce byte-identical output under R and G (`r9 diffrun`);
- the 2x2 of kernel and userland attributes every failure before it is filed.

**The GUI runs on the host.** Every program in the GUI census (02.7, 83 programs) runs under `9h` on macOS arm64 and on Linux amd64 and arm64. It runs unmodified: the same `a.out` bytes that run on 9front. It passes its scripted test with pixel-exact output against the same program on a real 9front kernel, or with a declared mask. Programs that need hardware a host cannot give are excluded by name, and there are three.

**Performance.** For R against G on amd64:
- the geometric mean of R's run time over about 40 benchmarks is no worse than G's;
- no benchmark is more than 5% behind;
- R's text is no more than 25% larger.

kencc is a simple optimizer, so R should usually win.

Compile time is reported against two baselines:
- **GCC 16 at `-O2`**, on the subset of units GCC can compile with `-fplan9-extensions`. This is the parent goal's 2x gate.
- **kencc itself**, which is very fast. Whether rucc can be twice as fast as `6c` is open question 1, and P0 measures it before anyone promises it (13.5).

**Versions.** `front` at the pin, the current release, and the previous release. A new release joins the day it is tagged.

**Architectures.** amd64 first, then arm64. 386 and arm are later.

## Settled decisions

**G is kencc, built by itself, from the same pin.** It is the `6c` and `7c` that 9front's own `mk install` produces from the pinned tree. The official release images are cross-checks, not the reference (04.2).

**rucc emits kencc objects, not ELF.** R has to be linked by the tree's own `6l` and `7l`, or the build is not unmodified. So rucc emits `.6` and `.7` files that those linkers read. rucc does not replace `6l` or `7l`. rucc also replaces `6a` and `7a` for the tree's `.s` files, but only from P6 on. Until then the tree's own assemblers build them, and the census counts those files as `ref-as` (07).

**rucc runs on the host, and the build runs in the guest.** In the guest's build namespace, `r9-cc` is bound over `6c` and `7c`. It is a Plan 9 program in `rucc-plan9`, built by G. It sends the compile to `r9 ccd` on the host. `r9 ccd` runs rucc, which reads the guest's files over 9P. Nothing in the tree knows (05).

**`9h` is a kernel, not a compatibility layer.** It is 9front's kernel: `/sys/src/9/port` unmodified, plus an out-of-tree arch directory `rucc-plan9/9h/{arm64,amd64}` for a paravirtual machine. It is built in the guest by R (or G), like any other kernel. A small host monitor, also called `9h` and written in Rust in `rucc-plan9`, runs it under Hypervisor.framework or KVM. The monitor gives each virtual processor one host thread and holds guest memory in one host mapping. It serves the devices as hypercalls: screen, mouse, keyboard, audio, disk, network, snarf and the host's files. The Plan 9 kernel and Plan 9 user code both run natively, in guest kernel mode and guest user mode (10).

drawterm copies about 30 files of port and edits them to run on the host. `9h` edits none and runs all of port. It can, because it keeps the kernel where port expects it: in kernel mode, with user memory at user addresses. The obvious alternative, running port on the host as ordinary threads, does not work on Apple Silicon. 10.1 explains why.

**The GUI is a kernel device.** `port/devdraw.c` is compiled unmodified, and `9h`'s `h9screen.c` turns `flushmemscreen` into a hypercall that updates a host window. The first claim is screen mode: one host window per `9h` session, with `rio` inside it. Rootless mode puts each `rio` window in its own host window. It is later work, and open question 4 asks whether it can be done without a different window system (11).

**The GUI oracle is pixels.** `rio` exports each window's image as `/dev/wsys/N/window`. `/dev/mousein` and `/dev/kbdin` accept scripted input. `memdraw` is software. Together these make a program's output an exact, comparable byte string (12.6).

**Compiler fixes go to rucc, tools go to `rucc-plan9`, reductions go to the corpora.** `rucc-plan9` contains:
- no compiler code;
- no 9front source;
- no patches;
- no copied 9front files.

`9h`'s arch directory is new code, licensed MIT like 9front, and written for this purpose. Where it is derived from drawterm, the file says so (14.1).

**`*_BRINGUP` delegations make a run ungraded.** `R9_BRINGUP=cc:<file>` sends one named unit to G's `6c`, and the census records it as `ref-c`. Any such delegation turns the run's first line into "UNGRADED". There is no other way to bypass rucc (14.4).

## The documents

| | | |
|---|---|---|
| 00 | this file | the goal, the settled decisions, the reading order |
| 01 | `01-research-2026.md` | 9front, its releases and sizes, kencc, prior art: drawterm, 9vx, plan9port, Inferno, gVisor, Noah |
| 02 | `02-the-goal.md` | the claims, stated precisely, and what does not count |
| 03 | `03-where-rucc-stands.md` | the gap inventory, probed against rucc 0.30.15, with sizes |
| 04 | `04-versions-references-and-machines.md` | pins and sets, G and how it is built, the builders |
| 05 | `05-build-system-mk-and-identity.md` | `mk`, `objtype`, the in-guest build, `r9-cc`, `r9 ccd` and 9P, the census |
| 06 | `06-the-plan9-c-dialect.md` | the dialect, the preprocessor, the pragmas, the type rules, the flags |
| 07 | `07-assembler-and-object-format.md` | `.6` and `.7` Prog streams, `6a`/`7a` syntax, signatures, `acid` output |
| 08 | `08-codegen-abi-and-runtime.md` | data model, kencc calling convention, `extern register`, runtime helpers, `a.out` |
| 09 | `09-kernels-and-boot.md` | the pc64 and arm64 kernels, configurations, boot under QEMU |
| 10 | `10-the-hosted-runtime-9h.md` | `9h`: unmodified port and new arch files in the guest, HVF and KVM machines, hypercalls and devices, the monitor |
| 11 | `11-gui-on-macos-and-linux.md` | screen and rootless modes, input, HiDPI, snarf, audio, fonts, case sensitivity |
| 12 | `12-tests-and-oracles.md` | `mk test`, the fixed point, `diffrun`, boot, the 2x2, the pixel oracle |
| 13 | `13-performance.md` | runtime benchmarks, GUI benchmarks, compile time against GCC 16 and kencc |
| 14 | `14-rucc-plan9.md` | the new repository, its commands, shims, rules and reports |
| 15 | `15-sibling-repos.md` | what each sibling repository gains, and when |
| 16 | `16-milestones.md` | P0 to P11, exit criteria, costs |
| 17 | `17-open-questions.md` | the ranked list, and when each must be answered |

Read 02, 03 and 16 first. Then read 06 and 08, because the dialect and the ABI are where Plan 9 differs most from anything rucc compiles today. Then read 10, because `9h` is the new kind of thing in this folder. The rest is reference for whoever picks up a milestone.
