# Kernels and boot

## 09.1 What `mk kernels` builds

`/sys/src/mkfile`'s `kernels` target runs `mk clean; mk install; mk clean` in `/sys/src/9` and `/sys/src/boot`. `/sys/src/9/mkfile` builds every directory in `ARCH`. Each kernel's mkfile sets its own `objtype`, so one `mk kernels` builds kernels for six architectures:

| Kernel dir | `objtype` | Compiler | In the claim |
|---|---|---|---|
| `pc64` | `amd64` | `6c` | yes: booted, tested, benchmarked |
| `arm64` (QEMU virt) | `arm64` | `7c` | yes: booted and tested |
| `bcm64` (Pi 3/4), `imx8` (Reform), `lx2k` (Honeycomb) | `arm64` | `7c` | built by rucc, linked, not booted (02.6) |
| `pc`, `xen` | `386` | `8c` | built by G's `8c` in both R and G until P10 |
| `bcm`, `kw`, `omap`, `teg2`, `cycv`, `zynq` | `arm` | `5c` | built by G's `5c` until P11 |
| `mt7688` | `spim` | `vc` | always G's compiler; outside the plan |
| `sgi` | `mips` | `vc` | always G's compiler; outside the plan |

`mtx` and `ppc` are commented out of `ARCH` at the pin.

`/sys/src/boot` builds the boot programs:
- `boot/pc`, the BIOS loader `9boot` and the `pbs` boot sectors. It is 16- and 32-bit x86 code with `objtype=386`.
- `boot/efi`, the EFI loader. It is built with both `8c` and `6c` for the 32- and 64-bit EFI images.
- `boot/qemu`, `boot/bcm`, `boot/reform`, `boot/zynq` and others for the boards.

**The census rule.** In R, `r9-cc` is bound over `6c` and `7c` only. Units compiled by `8c`, `5c` and `vc` go to G's compilers in both builds. They are recorded with origin `other-arch`, not `ref-c`. A graded run requires `ref-c = 0` over `objtype ∈ {amd64, arm64}`, and reports the `other-arch` count beside it. Each milestone that adds an `objtype` (P10 for 386, P11 for arm) moves that architecture's units from `other-arch` into the rucc count. So `mk kernels` is always the full documented command, and the claim's scope is stated by the census, not by removing directories from the build.

**Libraries and programs.** A kernel for `objtype=arm` links `/arm/lib/libc.a` and the rest, and its boot file system has programs of its `objtype`, such as `/arm/bin/paqfs`, `/arm/bin/auth/factotum` and the programs of `bootfs.proto`. `mk install` in an amd64 guest builds only `/amd64/lib` and `/amd64/bin`, and the first R run of `mk kernels` stopped at `/arm64/bin/paqfs`. Thus `r9-build` has the target `cross`: for each kernel `objtype` that is not the `objtype` of the guest, it runs `objtype=$o mk install` in `/sys/src` before `mk kernels`. That is the tree's documented way to cross-build, so it counts as a build command of 02.1. The units of these builds are `other-arch` too, except `arm64`'s, which are R's from P4 on.

## 09.2 How a kernel is built

`9/pc64/mkfile` is typical:
- `CONF=pc64`;
- `DEVS` comes from `../port/mkdevlist $CONF`;
- `PORT` lists the 30 port objects (`alarm`, `alloc`, `allocb`, `chan`, `proc`, `sysproc`, `segment`, `fault`, `page`, `qio`, `qlock`, …);
- `OBJ` is the arch objects (`l`, `cga`, `fpu`, `i8253`, `i8259`, `main`, `memory`, `mmu`, …);
- the configuration's generated files: `$CONF.c` from `../port/mkdevc` and `$CONF.rootc.c` from `../port/mkrootc`, which embeds the boot file system `bootfs.paq`;
- `LIB` lists `libmemlayer`, `libmemdraw`, `libdraw`, `libip`, `libsec`, `libmp`, `libc`, `libfis`, `libaml` and `libdtracy`.

The link is `$LD -o 9pc64 -T0xffffffff80110000 -R4096 -l $prereq`. `-l` means "no default libraries", and `-T` sets the kernel's text address (`KTZERO`, matching `mem.h`).

Rules that matter to rucc:
- **Some `pc` files are compiled in `pc64`** with `$CC $CFLAGS -I. -. ../pc/$stem.c`. `-.` turns off the include lookup in the file's own directory, so `../pc/foo.c` includes `pc64`'s `dat.h` and not `pc`'s. rucc's include order must honour `-.` exactly (06.4).
- **Headers are copied** from `../pc` into `pc64` by a rule (`PCHEADERS`). This writes in the tree, as the tree's own build always does. The overlay is thrown away (05.2).
- **The kernel is compiled with the same `CFLAGS=-FTVw`** as user space. There is no special flag for kernel code: no red-zone flag, no code model flag. There is also no code model for rucc to choose. rucc emits `SB`-relative addresses in the Prog stream (07.2), and `6l` picks the encoding: a sign-extended 32-bit absolute address fits both the kernel at `0xffffffff80110000` and user space at `0x200000`. A unit compiled once links into either.
- **The libraries are the same `.a` files** user space links, compiled by the same compiler. So R's kernel contains R's `libc` and `libmemdraw`, and the 2x2 (12.7) varies kernel and user space together with their libraries.

## 09.3 What the kernels need from rucc beyond user space

- **`extern register`** for `m` and `up` (06.9). On amd64, the assembly in `l.s` loads `R15` from the per-CPU `Mach` on every entry, and the C code reads `m->` through it. On arm64 it is `R27` and `R26`.
- **Exact struct layout** for every structure that `l.s` touches by offset. `l.s` includes no C header. It uses constants from `mem.h` and hand-written offsets such as `Ureg`'s fields and `Mach`'s `machno`, `splpc` and `proc`. A layout difference is a crash at the first trap. The `acid` comparison of 07.7 catches it before boot.
- **No floating point in the kernel** except where the kernel saves state (`fpu.c`). kencc does not generate SSE for integer code. rucc must not either: no SSE for struct copies, no vectorized loops. On each Plan 9 target, rucc keeps integer code in the general registers, in kernel units and user units the same.

  The detection rule was open question 8, and rule 2 is chosen. rucc does not use the vector registers for integer code in any unit of a Plan 9 target, so no path rule is necessary. The reason is that the kernels link `libsec` and `libc`, which are outside `/sys/src/9`.
- **`splhi` and `spllo` ordering.** These are assembly functions (`l.s`), so they are ordinary calls to rucc and therefore barriers. rucc may not move memory accesses across any call (08.7).
- **`getcallerpc`** in `lock` and `qlock` (08.2).
- **`setlabel` and `gotolabel`.** These are `l.s` functions that save and restore SP and PC (`Label`), like `setjmp`. The kencc convention makes them safe: no register survives any call (08.2), so nothing lives in a register across `setlabel` that a second return could find stale. rucc must not add an exception, such as an interprocedural register allocation that keeps a value in a register across calls to functions it can see do not clobber it. The calls into `.s` functions are opaque, and the rule is that no call preserves a register.

## 09.4 Boot under QEMU

**amd64.** `r9 image` builds a disk:
1. Start from the build overlay's root, which is R's or G's installed tree.
2. Write the kernel `9pc64` (R's or G's).
3. Write `plan9.ini` with `bootfile=9pc64`, `monitor=vesa`, `vgasize=1024x768x32` and `mouseport=ps2intellimouse`.
4. Write 9front's own `9boot` loader from the same build. Until P10 that is G's, because it is `386` code.

QEMU boots it with the machine of 04.4. `r9-boot` is an `rc` script in the image. `/rc/bin/termrc` runs `/rc/bin/termrc.local` if it exists (line 30 at the pin), and `r9 image` writes that file to run `r9-boot`. No file of the tree changes; `termrc.local` is not in the tree. `r9-boot` does the following:
- reports over the serial console;
- starts `rio` with a fixed screen;
- runs the smoke suite;
- writes `/dev/screen` to the result disk;
- halts with `fshalt`.

Boot passes if:
- the console shows the same sequence of lines as G's boot, after a normalization that removes times and addresses;
- the smoke suite passes;
- `rio` draws a screen equal to G's screen, apart from the clock and the `stats` area if shown.

**arm64.** The `qemu` kernel boots on `-machine virt` with 9front's `boot/qemu` `u-boot`-free path. The release's arm64 qcow2 shows the arguments. It boots to a serial console with no graphics (01.5). The smoke suite runs over the serial console. GUI programs for arm64 are tested in two ways:
- on Q, through drawterm connected to the guest's `cpu` listener. drawterm's own `devdraw` renders, and the GUI scripts read the window images through `/mnt/term/dev/draw` from the guest side;
- on H, through `9h` on the M4.

The drawterm path is a G-side reference only: drawterm is a host program built by the host C compiler, not part of the claim.

**Timeouts.** The timeouts are 120 s to `rio` on amd64 KVM, 60 s to a shell on arm64 HVF, and 600 s to a shell on arm64 TCG. P0 replaces these with three times the measured median.

## 09.5 The smoke suite

These are `rc` scripts in `rucc-plan9/guest/smoke/`. They exercise the kernel paths every program depends on:
- `rfork` with every flag;
- `exec` of a 2 MB binary;
- `segattach` shared memory;
- `rendezvous`;
- semaphores (`semacquire`, `tsemacquire`);
- notes and `alarm`;
- pipes;
- `mount` of a `9P` server (`ramfs`);
- `bind` unions;
- `srv`;
- `exportfs` over a pipe;
- `dial` to `tcp!localhost`;
- `factotum` with `p9sk1` against a local `authsrv`;
- the `/proc` files;
- `ps`;
- `kill`;
- `sleep`;
- the `#c/time` ticks.

Each script prints `ok name` or `FAIL name: reason` on the console. The suite has about 60 cases, and it is the first gate on any R kernel. It is the same in G, so a failure in G is a rig bug.
