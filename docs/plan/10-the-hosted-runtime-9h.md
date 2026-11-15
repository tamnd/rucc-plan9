# The hosted runtime: `9h`

`9h` runs unmodified 9front binaries on macOS and Linux, each in a host window, at native speed. It is two things:
- **The `9h` kernel.** It is 9front's `port/`, the tree's own arch files for `arm64` or `pc64`, and a small set of new files for a paravirtual machine: a configuration and device drivers. It is built in the guest by `7c`/`6c` like any other kernel, so R and G each build one.
- **The `9h` monitor.** It is a Rust program in `rucc-plan9` that creates the machine with Hypervisor.framework (macOS) or KVM (Linux), loads the kernel, and serves its devices. It is the only host code in the picture.

This document describes both. Document 11 describes the window, input and audio back ends the monitor uses.

## 10.1 Why the kernel runs in the guest

The first design compiled `port/` for the host, ran kernel processes as host threads, and ran only user code in the guest. That was drawterm's model taken further. It does not work, for three reasons, each enough on its own:
1. **Port dereferences user pointers.** After `validaddr`, port and its devices read and write user memory by address: `sysread` passes the user's buffer to the device's `read`, and `devcons`, `devpipe`, `devdraw` and `qio` copy straight into it. On the host, the user's address space would have to be mapped at the same virtual addresses in the kernel's host process.
2. **The pages do not fit.** Plan 9 pages are 4 KiB on both `arm64` and `amd64` (`BY2PG`, `mem.h`). macOS on Apple Silicon maps host memory in 16 KiB pages. Four consecutive 4 KiB user pages are generally four unrelated physical pages, and one 16 KiB host page cannot show them. So the host cannot mirror a user address space on the M4, which is the host the claim needs most.
3. **SMP.** With several processors, each runs a different process, and one host address space can mirror only one of them. Separate host processes per processor would need port's whole data and heap in shared memory at fixed addresses, which is a kernel of its own.

drawterm avoids all three by editing port: its `kern/` copies about 30 files and changes how they reach user memory, and it runs only its own built-in programs, never Plan 9 binaries. `9h` must run all of port unmodified, so it puts the kernel where port expects it: in kernel mode, with user memory at user addresses. The hypervisor provides exactly that, and the guest's MMU translates 4 KiB pages whatever the host's page size.

What this costs:
- The guest has its own physical memory, which is one host allocation.
- Devices are hypercalls instead of function calls.

What it gains:
- `port/` and the tree's arch code run exactly as on hardware.
- `9h` needs no rucc host mode, so the `-mlong32` flag of an earlier draft is gone (03.17, 08.8).
- The `9h` kernel is a fourth R-built kernel to test.

## 10.2 The `9h` kernel

**Sources.** `rucc-plan9/9h/arm64/` and `rucc-plan9/9h/amd64/` contain only new files:

| File | Purpose |
|---|---|
| `9h` | the kernel configuration (the `qemu` and `pc64` files are the models) |
| `h9.h`, `h9.c`, `h9l.s` (amd64) | the hypercall interface (10.5): numbers, structures, and the call. On arm64, `h9call` in `h9.c` uses `hvccall` of the tree's `l.s`, so arm64 needs no assembler file. `h9.c` also has empty `pciintrenable` and `pciintrdisable`, which `gic.c` calls, because the configuration has no PCI driver. On amd64, `h9call` is in `h9l.s`, because `h9.s` and `h9.c` would make the same object. The amd64 `h9.c` has the three functions of the screen files that pc64 `main.c` and `pc/bootargs.c` call: `bootscreeninit`, `rgbmask2chan` and `vesabootscreenconf` |
| `uarth9.c` | console output and input, as a uart of port's `devuart`. `mkdevc` finds a uart driver by the prefix `uart`, and `kbdfs` reads the console as `/dev/eia0`, as on the `qemu` kernel. On arm64, `uartconsinit` makes it the console. On amd64, pc64 `main.c` calls `i8250console`, so the amd64 file has that name for the same function, and the configuration has no `uarti8250` |
| `h9screen.c` | `screen.c` for `devdraw`: framebuffer, `flushmemscreen`, cursor, resize |
| `h9mouse.c` | absolute mouse through port's `absmousetrack` |
| `h9kbd.c` | `#b/kbd`: key-down and key-up rune events, the input `kbdfs` prefers when it exists (11.4) |
| `audioh9.c` | an `audio` card for port's `devaudio` |
| `sdh9.c` | a disk for port's `devsd`, backed by a host file. The units are `sdH0` to `sdH3` |
| `etherh9.c` | an Ethernet for port's `devether` |
| `devhost.c` | `#Z`: the snarf buffer, the host file system channel `hostfs`, and `ctl` (10.5). The three are done |

The files are about 2,000 lines on each architecture. They use only interfaces port and the tree's arch code already export. Where one is derived from a tree file (`h9screen.c` from `bcm/screen.c`, `sdh9.c` from `sdram.c`), its header says so. `#Z` is unused at the pin, and `r9 9h verify` checks that it still is at each pin.

**Building.** The kernel is built in the guest, with the target `9h` of `r9-build`, as in `r9 build --targets 'cross 9h'`. `r9 build` puts `9h/arm64` and `9h/amd64` of the repository on the input disk, and the guest copies them to `/usr/glenda/r9/9h`. For each architecture, the target runs this in a new namespace:
```rc
cd /sys/src/9/arm64 && bind -ac /usr/glenda/r9/9h/arm64 . && mk clean && mk 'CONF=9h'
```
The union makes the new files visible beside the tree's arm64 files. Because the bind has `-c`, each file that mk makes goes to `/usr/glenda/r9/9h/arm64` and not to the tree. A bind with `-a` and no `-c` gives a union where a create fails. The kernels go to `/tmp/r9/9h/arm64` and `/tmp/r9/9h/amd64`, and `r9 build` collects them. The kernel of an architecture other than that of the guest needs the cross build of that architecture, which the target `cross` makes. The tree's own mkfile builds the kernel: `CONF=9h` selects the configuration, `../port/mkdevc` generates `9h.c`, and the same `OBJ` list builds `l.s`, `mmu.c`, `trap.c`, `fpu.c`, `clock.c`, `gic.c`, `main.c` and the rest unchanged. On amd64, the same happens in `/sys/src/9/pc64`, with pc64's `l.s`, `mmu.c`, `trap.c`, `apic.c`, `archacpi.c` and `i8253.c`, and the files of `pc/` that the pc64 mkfile compiles by name. The configuration leaves out every PCI device driver, and the VGA, USB, audio and keyboard devices. It keeps `pcipc.c`, which finds no PCI bus, and `devkbd.c`, because `archgeneric.c` calls its `i8042reset`. The new files come in as `link` and `misc` entries.

**Unmodified.** `r9 9h verify` reads the R build's `compile.jsonl` for the `9h` kernel and checks three things:
- every unit's source path is in the tree or in `rucc-plan9/9h/`;
- every tree source has the pin's hash;
- nothing in `9h/` has the name of a tree file in `port/`, the arch directory, or `pc/` for amd64, so a union cannot shadow one.

The build also reads files that it makes: `$CONF.c`, `errstr.h`, the `.i` files, and on amd64 the headers that the pc64 mkfile copies from `pc/`. A copied header must have the hash of the pin's file in `pc/`.

The `9h` kernel is built by R and by G, and both run on the same monitor. So the GUI results have the same 2x2 as boot (12.7): R/H means R's binaries on R's `9h` kernel.

## 10.3 The arm64 machine (HVF and KVM)

The monitor builds the machine the tree's `arm64` code already knows: QEMU's `virt` layout, minus PCI. Its physical map:

| Range | What |
|---|---|
| `0x08000000` | GICv3 distributor; redistributors at `0x080a0000` |
| `0x09000000` | reserved for the `9h` kernel, where the console is a hypercall. Until that kernel exists, the monitor puts a PL011 here and a PL031 at `0x09010000`, so the `qemu` kernel of the tree can run (10.3.1) |
| `0x40000000` | RAM, 1 GiB by default (`-m`), one host allocation mapped with `hv_vm_map` or `KVM_SET_USER_MEMORY_REGION` |
| top of RAM | the screen's framebuffer is ordinary RAM the kernel allocates; the monitor reads it through the same host mapping |

**Start-up.**
1. The monitor reads the kernel `a.out` (`7l` output: magic, text, data, bss, entry).
2. It copies the segments to their physical addresses (link address minus `KZERO`).
3. It writes a device tree with `/memory`, `/chosen/bootargs` and the GIC.
4. It starts vCPU 0 at the entry point in EL1 with the MMU off and `X0` pointing at the device tree.

`l.s` handles EL1 entry (`svcmode` accepts EL1 and EL2), and `bootargs.c` reads `*maxmem` and the boot arguments from the device tree. Secondary processors start through PSCI `CPU_ON`, which `main.c`'s `mpinit` sends with `hvccall`. At the start, the monitor makes one vCPU for each processor of `--cpus`, each on its own host thread, in the order of the processor numbers. vCPU 0 starts at the entry point, and the others wait. The monitor answers `CPU_ON` by starting the vCPU of the target processor at the address and with the context of the call, and `AFFINITY_INFO` with the state of that processor. `CPU_OFF` stops the vCPU of the caller, and the session ends when each vCPU is off. `SYSTEM_OFF` and `SYSTEM_RESET` end or restart the session.

**Hypervisor.framework.**
- The monitor creates one VM per session with `hv_vm_create`, and one vCPU per host thread with `hv_vcpu_create`. A vCPU can only be run by the thread that created it.
- The GICv3 is the framework's, through `hv_gic_create` (macOS 15). `gic.c` programs it as on QEMU. Without it, the monitor would emulate the distributor and redistributors itself, about 1,000 lines. P7 decides after measuring the in-framework GIC; macOS 15 is already the minimum (02.7).
- The generic timer is the guest's own virtual timer. When it fires, `hv_vcpu_run` returns `HV_EXIT_REASON_VTIMER_ACTIVATED`. The monitor asserts the timer's PPI in the GIC and unmasks the timer when the guest acknowledges it.
- `HVC #0` from EL1 exits as an exception with class `0x16`. That is the hypercall path (10.5), and PSCI uses it too.
- To interrupt a running vCPU for a device interrupt, the monitor calls `hv_vcpus_exit`, injects the interrupt and resumes.

### 10.3.1 First boot on the M4

The first test of the monitor is the `qemu` kernel of the tree, `9qemu.u` from the G arm64 image, with no change. It boots on the M4 (macOS 15.8) to the `bootargs` prompt of `bootrc`, and `!rc` at that prompt gives a shell on the terminal. The boot showed these facts about the framework and the kernel:

- With `hv_gic_create`, the framework delivers the PPI of the virtual timer and handles `WFI` itself. The boot had no `HV_EXIT_REASON_VTIMER_ACTIVATED` exit and no `WFI` exit. The monitor keeps both exit paths, because a configuration without the framework GIC needs them.
- `ID_AA64DFR0_EL1.PMUVer` is 0 by default. With that value, a PMU access in the guest takes an Undefined exception in the guest, and the monitor does not see it. The header of `hv_vm_config.h` says that such an access exits to the monitor when EL2 is off, but on the M4 it does not. `clockinit` writes `PMCR_EL0`, so the monitor sets `PMUVer` to 1 (PMUv3), and the framework then emulates the PMU.
- `bootargs.c` copies `/chosen/bootargs` over the `plan9.ini` at `CONFADDR` when the property exists, even when it is empty. The monitor puts the property in the device tree only when `--bootargs` is given. Without the `plan9.ini`, `$console` is not set, `kbdfs` does not open `/dev/eia0`, and the console gets no input.
- The PCI scan of `pciqemu.c` reads the ECAM window at `0x3f000000`. There is no device there, so each read gives all ones, and the scan finds no PCI device. That is the result the kernel expects from an empty bus.

The boot with more than one processor showed these facts:

- The framework gives the redistributor frames in the order in which the vCPUs are made, at `0x080a0000` plus 128 KiB for each vCPU. `GICR_TYPER` has the vCPU number as the processor number, the MPIDR of the vCPU as the affinity, and the Last bit on the frame of the last vCPU only. `getrregs` in `gic.c` finds the frame of a processor by its processor number. Thus the monitor makes all the vCPUs at the start, in order, and does not make a vCPU when `CPU_ON` comes.
- A 32-bit read of `GICR_TYPER` gives 0, and only a 64-bit read gives the value. The `gic.c` of the pinned tree reads the register as 64 bits, and the kernel boots on 4 processors. The `9qemu.u` of the arm64 image reads the low 32 bits, does not find its frame, and stops in `getrregs: no re-distributor for cpu 1`. A copy of that kernel with the two loads of `getrregs` changed to 64-bit loads boots on 1, 2 and 4 processors. Each processor takes about 800 timer interrupts in its first 8 seconds, as `/dev/sysstat` shows, and `reboot` stops each processor and ends the session.
- The `9qemu.u` of the arm64 image calls `CPU_ON` with the 32-bit number `0x84000003`, and the pinned tree uses the 64-bit number `0xC4000003`. The monitor accepts the two numbers, as QEMU does.

`9h run --kernel 9qemu.u --ini plan9.ini --cpus 4` starts this test. `H9_TRACE=1` logs each exit, and it logs the state of the timer and of the GIC each 500 ms.

**KVM on Linux arm64.** This is the same machine, with KVM's in-kernel vGICv3 (`KVM_DEV_TYPE_ARM_VGIC_V3`) and timer, and `KVM_EXIT_HYPERCALL` for `HVC` with SMCCC filtering (`KVM_ARM_VM_SMCCC_FILTER`, Linux 6.4 and later). PSCI is KVM's own.

## 10.4 The amd64 machine (KVM)

The monitor builds a minimal PC that pc64's code accepts:
- RAM from 0, with the usual hole at `0xa0000`–`0x100000`;
- KVM's in-kernel interrupt controllers (`KVM_CREATE_IRQCHIP`: local APIC, I/O APIC and PIC) and PIT (`KVM_CREATE_PIT2`);
- ACPI tables written by the monitor: RSDP, XSDT, MADT and FADT, with no AML beyond an empty DSDT. `archacpi.c` reads them to find the processors and the I/O APIC.

**Start-up.** `9pc64` has a multiboot header, which `l.s` accepts. The monitor loads the kernel as a multiboot loader would. It starts vCPU 0 in 32-bit protected mode with `EAX = 0x2BADB002` and `EBX` pointing at a multiboot information block. That block carries the memory map and the command line, which holds the `plan9.ini` variables. `l.s` then enables long mode itself, as it does after 9boot. Application processors start through the local APIC's INIT and SIPI, which KVM's in-kernel APIC handles.

Port I/O and MMIO exits to anything but the hypercall port are logged and return all ones, so a stray probe of hardware that is not there behaves as an empty slot. P7 runs the `9h` kernel with these logs to make sure the configuration has no such probes. The hypercall is a 32-bit `OUT` to I/O port `0x509`, which the monitor reserves. It exits as `KVM_EXIT_IO`, and the monitor reads the arguments with `KVM_GET_REGS`. `VMCALL` is not usable: KVM handles it in the host kernel and returns `-KVM_ENOSYS` for numbers it does not know, except one it forwards for confidential guests.

**Reset and power off.** When the kernel exits or reboots, `archreset` of the tree writes 6 to port `0xcf9`, the reset control register of the PC. The monitor ends the session with a reset when it sees a write with bit 2 set. The FADT has no reset register and no PM1 blocks, so the kernel uses that port, and `ctl` of `#Z` with `quit` turns the machine off, as on arm64.

Intel Macs (HVF on x86) could run the same machine with `hv_vm_create` and the x86 framework, but the framework has no in-kernel APIC there, so the monitor would emulate it. That is not planned (02.6).

## 10.5 Hypercalls and devices

**The call.** One instruction: `HVC #0` on arm64, `OUT` on amd64. The call number is in `R0`/`AX`, and up to five arguments are in the next argument registers. On arm64 the numbers are SMCCC function IDs in the vendor-specific hypervisor range, so PSCI and `9h` calls share one path, and KVM's SMCCC filter forwards exactly that range to the monitor. The result comes back in the same register. Calls never block the guest for long. A call that would wait, such as a disk read, starts the operation and returns. The monitor raises the device's interrupt when it is done. Bulk data goes through guest memory: the guest passes physical addresses, and the monitor reads or writes them through its mapping. The monitor checks every address against RAM before using it.

| Device | Calls | Interrupt | Monitor side |
|---|---|---|---|
| console | `putc`, `puts` | console input | the monitor's terminal, or a log file |
| screen | `screen.info`, `screen.flush(rect)`, `screen.cursor(img, hot)`, `screen.mode(w,h)` | resize | blits the framebuffer rectangle into the host window (11) |
| mouse | none | an event ring in a shared page | host pointer to `absmousetrack(x, y, buttons, msec)` |
| keyboard | none | an event ring | host key events to `r`/`R` rune messages (11.4) |
| audio | `audio.start(ring)`, `audio.volume` | buffer done | CoreAudio or PipeWire (11.6) |
| disk | `sd.info`, `sd.read`, `sd.write`, `sd.flush` | none | a host file, in requests of at most 64 KiB |
| ether | `ether.tx(pa, n)`, a receive ring | receive | user-mode NAT (as QEMU's `-netdev user`), or `vmnet` on macOS and a tap device on Linux when configured |
| `#Z/snarf` | `snarf.size`, `snarf.get(pa, n, off)`, `snarf.put(pa, n)` | none | the host clipboard, text only, UTF-8 |
| `#Z/hostfs` | `fs.reset`, `fs.write`, `fs.read` | none | the monitor's 9P server over a host directory (`h9-9p`) |
| `#Z/ctl` | `ctl(pa, n)` | none yet | window title and quit now, and scale with the screen (11) |
| clock | none | none | the generic timer or the TSC, and an RTC value in the device tree or ACPI |

**The numbers.** The first calls are those of the console. `9h/arm64/h9.h` and `h9-dev/src/h9.rs` have the same numbers:

| Number | Name | Arguments | Result |
|---|---|---|---|
| `0xC6000000` | `H9version` | none | the revision of the interface, 1 |
| `0xC6000001` | `H9consputc` | the byte | 0 |
| `0xC6000002` | `H9consputs` | the physical address and the count, at most 64 KiB | the count written, or -2 for an address that is not in RAM |
| `0xC6000003` | `H9consgetc` | none | the next input byte, or -1 |
| `0xC6000010` | `H9sdinfo` | the unit | the number of 512-byte sectors, or -1 when the unit does not exist |
| `0xC6000011` | `H9sdread` | the unit, the first sector, the count, and the physical address | the count read, at most 128 |
| `0xC6000012` | `H9sdwrite` | the unit, the first sector, the count, and the physical address | the count written, at most 128 |
| `0xC6000013` | `H9sdflush` | the unit | 0 |
| `0xC6000020` | `H9fsreset` | none | 0, or -1 when the monitor serves no host directory |
| `0xC6000021` | `H9fswrite` | the physical address and the count | the count, or -3 for a message size that is not correct |
| `0xC6000022` | `H9fsread` | the physical address and the count | the count of reply bytes copied, 0 when none waits |
| `0xC6000030` | `H9snarfsize` | none | the length of the host clipboard, at most 1 MiB, which the monitor keeps for `H9snarfget` |
| `0xC6000031` | `H9snarfget` | the physical address, the count and the offset | the count copied from that offset |
| `0xC6000032` | `H9snarfput` | the physical address and the count, at most 1 MiB | the count, or -3 when it is too large |
| `0xC6000040` | `H9ctl` | the physical address and the count of a request, at most 256 bytes | the count, or -3 for a request that the monitor does not know |

A number that no device has gives -1, and the monitor logs it once. The console input interrupt is SPI 16 (INTID 48). It is a level interrupt, and it stays high while an input byte waits.

A disk call that fails gives -2 for an address that is not in RAM, and -3 for a sector outside the disk, a write to a read-only unit, or an error of the host. The disk calls do the transfer before they return. This is different from the general rule above, for two reasons. A request is at most 64 KiB, which takes less than a millisecond from a host SSD. Also, the driver needs no interrupt and no wait, so it is as small as `sdram.c`. If a profile shows that the guest waits too long in these calls, a later revision can add a form that starts the transfer and raises a done interrupt. The monitor does not set `O_DIRECT` or `F_NOCACHE` yet, so the host caches the disk file. `H9sdflush` calls `fdatasync` on the file.

The server of `#Z/hostfs` answers each whole 9P message in the `H9fswrite` call that gives its last byte. Thus a reply waits by the time `devmnt` reads for it, and `devhost.c` needs no interrupt. A read that finds no reply sleeps until the next write. `devhost.c` opens `hostfs` for one user at a time, and each open starts a new session with `H9fsreset`. The option `--hostfs DIR` serves DIR, and `--hostfs DIR,ro` serves it read-only. `--user NAME` gives the owner of the files, `glenda` by default.

`#Z/snarf` gets the host clipboard again at each read at offset 0, because `rio` keeps its snarf file open and seeks to 0 for each paste. The text written to a snarf file goes to the host clipboard when the file is closed. The monitor uses the clipboard programs of the host: `pbpaste` and `pbcopy` on macOS, and `wl-paste` and `wl-copy`, `xclip` or `xsel` on Linux. A program is simpler than the clipboard calls of AppKit or of the display server, and 10 ms for a paste is not a problem. With `--snarf memory`, or on a host with no such program, the snarf stays in the monitor. Thus the clipboard needs no interrupt: the plan had one for a change of the host clipboard, but `rio` reads again at each paste.

`#Z/ctl` takes one request in each write. `quit` ends the session, as `SYSTEM_OFF` does. `title TEXT` sets the title of the host terminal, with the escape of xterm, when the console is a terminal. The monitor removes the control characters from TEXT first, because the guest gives it. With the screen (11), the title goes to the window, and `scale` is added.

The option `--disk FILE` adds a unit, and `--disk FILE,ro` adds a read-only unit. The first `--disk` is `sdH0`.

**The host's files.** The monitor serves a host directory as 9P2000 on the `#Z/hostfs` channel. The boot script mounts it on `/mnt/term`, as `drawterm` does. Host names are mapped as `drawterm` maps them:
- names that differ only by case on a case-insensitive host are exposed as they are, and refused on create;
- `/` and NUL cannot occur;
- host permission bits map to Plan 9's, and the owner is the session's user.

`/mnt/term` is not the root. The root is the disk (10.6), so case collisions in the tree never reach the host file system (11.8).

**Snarf.** `rio` reads and writes `/dev/snarf` if it exists (`rio.c:178`, `:248`). On native 9front it does not. `9h`'s boot script binds `#Z` into `/dev`, so unmodified `rio` exchanges its snarf buffer with the host clipboard. `rio` reads `/dev/snarf` again when it takes the snarf from a window, so a host copy shows up on the next paste.

## 10.6 Boot and the session

A `9h` session is `9h run --kernel=9h.r --disk=root.r.img --scale=auto`. The disk is the gefs root of a build (R's or G's), made by `r9 image` from the build's installed tree (09.4). Its `plan9.ini` variables go in the device tree or the multiboot command line:
- `bootargs=local!/dev/sdH0/fs`;
- `user=glenda`;
- `mouseport=h9`, `monitor=h9`;
- `service=terminal`.

The boot is 9front's own: `bootrc` from the kernel's `bootfs.paq`, then `/rc/bin/termrc`. `9h` adds `/rc/bin/termrc.local` to the disk, as `r9 image` does for Q (09.4). It is the session's only addition, and it does three things:
- `bind -a '#Z' /dev` and `mount -c /dev/hostfs /mnt/term`;
- it starts `rio` with the screen the window has;
- it starts the program the user asked for, if any (`9h run … -- acme`), in the first window.

From power-on to `rio` drawing, `9h` should take under two seconds on the M4. That is a target, measured at P7, not a claim.

## 10.7 Time and determinism

The guest reads time from the generic timer (arm64) or the TSC (amd64), which run in real time. That is right for use, and wrong for a pixel test of `clock` or a `doom` timedemo. GUI scripts (12.6) handle it this way:
- `clock`, `stats` and anything else that draws the time are masked;
- `doom -timedemo` measures frames, not seconds, and draws the same frames whatever the speed;
- scripts wait until the program is quiet, which `r9 gui` detects by the window image not changing for 500 ms, not by sleeping.

A virtual clock mode in the monitor, which advances guest time only by a fixed step per scheduler tick, would make more programs exact. It is not planned until the masked count shows it is needed (17).

## 10.8 The monitor

`rucc-plan9/9h/monitor/` is a Rust workspace:

| Crate | Purpose | Estimate |
|---|---|---|
| `h9-vm` | the VM abstraction: memory, vCPUs, exits, interrupts | 1,500 lines |
| `h9-hvf` | Hypervisor.framework back end (arm64) | 1,500 |
| `h9-kvm` | KVM back end (arm64 and amd64), ACPI and multiboot for amd64 | 2,500 |
| `h9-dev` | the devices of 10.5, and the hypercall dispatcher | 3,000 |
| `h9-9p` | the 9P server for `/mnt/term` (shared with `rucc-9p`'s message code) | 1,200 |
| `h9-gui` | the window, input and audio back ends (11) | 4,000 |
| `9h` | the command line, the session, logging and the test hooks | 800 |

That is about 14,000 lines. The monitor uses no QEMU code. It may use `kvm-ioctls` and `kvm-bindings` (Apache 2.0/MIT). Its HVF bindings are its own, because the framework is small.

**Test hooks.** `9h run --script=<file>` is the GUI test driver's entry (12.6). It injects input into the same rings the host window feeds, reads `/dev/wsys/N/window` back through `/mnt/term`, and runs headless with no window. Headless is what CI on gpc and server1 uses. On Linux it needs no display server at all, because the monitor's screen device does not need a window to exist.

The first form of `--script` drives the console only, because the screen does not exist yet. Its steps are `wait TEXT`, `send TEXT`, `timeout SECS` and `quit`. `r9 9h boot` and `r9 9h smoke` do not use a script. They read and write the console on the standard input and output of the monitor, with the console driver of `r9 boot`, so that a boot under QEMU and a boot under `9h` give the same `steps.json`. The steps of the GUI (`window`, `mouse`, `keys`, `snap`, `quiet`) come with the screen, in the same parser. The exit status is 0 when the script passes, and 3 when it fails.

## 10.9 What `9h` does not do

- **It does not run another architecture.** An arm64 host runs arm64 9front, and an amd64 host runs amd64. Running amd64 9front on the M4 would need an emulator, which is QEMU's job (17, question 5).
- **It does not run without a hypervisor.** On Linux without `/dev/kvm`, such as nested CI without nesting, `9h` exits with an error naming the missing device. Such a host can run Q under QEMU TCG with drawterm. That is not H, and it is not in the claim (17).
- **It is not rootless.** Each session is one host window with `rio` inside it. Rootless mode, with each `rio` window as a host window, is open question 4 (11.7).
- **It does not emulate hardware.** There is no VGA, PS/2, PCI, USB or virtio. A program that opens such a device (`aux/vga`, `nusb`, `aux/realemu`) fails under `9h` as on a machine without one. None of the 80 GUI programs in the claim does (02.7).

## 10.10 Checks

| Claim | Check |
|---|---|
| The `9h` kernel's tree sources are unmodified | `r9 9h verify` |
| The `9h` kernel boots R's and G's roots to `rio` on each host | `r9 9h boot --host=m4,gpc` |
| The smoke suite of 09.5 passes under `9h` | `r9 9h smoke` |
| The four kernel and user-space pairs agree on the GUI suite | `r9 gui --kernel=R,G --user=R,G` (12.7) |
| No device probe reaches an empty port or address | `r9 9h boot --strict` (unknown exits are fatal) |
| The monitor rejects guest addresses outside RAM | `cargo test -p h9-dev` fuzz targets |
