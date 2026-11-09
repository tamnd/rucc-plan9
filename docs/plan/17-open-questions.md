# Open questions

Each question says:
- what we believe now;
- what goes wrong if the belief is wrong;
- how to find out;
- the latest milestone by which it must be settled.

A question past its deadline blocks that milestone. The numbers are stable, because other documents cite them. The table ranks the questions by how much of the plan changes with the answer.

| Rank | Question | Deadline | If the belief is wrong |
|---|---|---|---|
| 1 | 2. Whether G is its own fixed point | P0 | 12.4's strongest oracle is lost, and P3's exit criterion changes |
| 2 | 11. The GIC on HVF | P7 | P7 gains about 1,000 lines and two weeks |
| 3 | 16. When kencc is wrong | P3 | `diffrun` and `mk test` failures are misfiled as rucc bugs, or the reverse |
| 4 | 10. The 9P cost of a compile | P1 | R's build wall time is several times G's, and `r9 ccd` needs a different sysroot |
| 5 | 8. The kernel's SIMD rule | P5 | a kernel that saves no SIMD state is corrupted by rucc's code |
| 6 | 1. Twice as fast as `6c` | P9 | the compile-time goal against kencc is dropped or redefined |
| 7 | 9. 9front's arm kernels on QEMU `virt` | P11 | arm is built but never booted |
| 8 | 14. A virtual clock in `9h` | P8 | fewer than 60 of 80 GUI programs are exact or masked |
| 9 | 6. An arm64 Linux builder | P8 | `9h` arm64 on Linux is not claimed |
| 10 | 7. The weekly pin | P10 | `front` grades a stale commit, or churns |
| 11 | 13. Linux without KVM | P7 | none for the claim; some users cannot run `9h` |
| 12 | 3. Strict aliasing | none | none for the claim; a performance option is lost |
| 13 | 4. Rootless mode | P11 | none for the claim |
| 14 | 15. Reporting upstream | P1 | none for the plan; a lot for how it is seen |
| 15 | 5. Other architectures on a host | none | none for this goal |
| 16 | 12. Intel Macs | none | none for this goal |

## 1. Whether rucc can compile twice as fast as `6c`

**Belief.** It cannot, at `-O2`.
- `6c` is one pass with no SSA and no optimizer. It compiles a typical unit in about 5 ms (05.4).
- Twice as fast means about 2.5 ms per unit, including rucc's optimizer and the `.6` writer.
- The parent goal's "2x faster" was written against GCC. The gate in 02.5 is therefore against GCC 16 on the 685 units it compiles (13.5), and kencc is reported only.

**Risk.** If the goal is read literally as "twice as fast as the system's own compiler", the plan fails it by design. A rucc tuned to win would drop the optimizer, and then it would lose the runtime gate, which is the more important one.

**How to find out.**
1. P0 measures `6c` per unit (16.1).
2. P1 measures rucc's front end on the same units with `-fsyntax-only`.
3. P9 measures rucc at `-O0` and `-O2` against `6c` (13.5).

If rucc at `-O0` is within twice `6c`'s time, a fast `-O0` mode for edit-compile loops is a reasonable goal, and 02.5 says so. If it is not, 02.5 records that kencc is the faster compiler and why.

**Deadline.** P9.

## 2. Whether G is its own fixed point

**Belief.** kencc's output depends only on its input, so building the tree twice with G gives byte-identical objects and binaries. 12.4's fixed point test (Rk equals G) rests on this. It does not hold for every file:
- **Archives.** `ar` records a date for each member, so `libc.a` differs by date. `r9 objdiff --tree` compares archives member by member.
- **Generated files.** A generator that writes the date, the host name or a process id differs too. Each such file is listed in the comparison's exclusions, with the line that varies.

Apart from those, we believe the objects and the linked binaries are identical.

**Risk.** Some sources of difference would make the fixed point a noisy test, and a noisy test gets ignored:
- an uninitialized buffer written to an object (padding in a record, for example);
- symbol order taken from a hash of pointers;
- the guest's time leaking into `6l`'s output.

**How to find out.** In P0:
1. Build G four times: twice in one guest, and twice in fresh guests (16.1).
2. Diff every object, archive member and binary.
3. Explain each difference and either exclude it, with its reason, or fix it in the comparison.

A difference with no explanation is a finding against 9front, reported by a person (question 15).

**Deadline.** P0. If the fixed point does not hold, 12.4 is redesigned to compare after normalization before P3 relies on it.

## 3. Whether the tree is clean enough for strict aliasing

**Belief.** kencc does no type-based alias analysis, so nothing in the tree has been tested against it. rucc therefore runs Plan 9 targets with strict aliasing off (08.7). Many places pun between `uchar*` and wider types, and between `Block` payloads and headers. Most of that is allowed through character types, but not all.

**Risk.** None for the claim, because graded runs never enable it. It costs rucc some performance in loops over typed arrays, against nothing, since kencc has none either.

**How to find out.** After P9, if a benchmark needs it:
1. Build user space with `-fstrict-aliasing` and run `mk test` and `diffrun`.
2. Use rucc's aliasing sanitizer, if one exists by then, on the failing programs.

**Deadline.** None. It is an option for later.

## 4. Rootless mode

**Belief.** Rootless mode, with each `rio` window as its own host window, can be built in two ways (11.7):
1. a `rio` replacement in the guest, which is a change to the tree's programs and outside the "unmodified" rule unless shipped as a separate program;
2. a debug hypercall that reads port's `devdraw` screens, which ties `9h` to `devdraw.c`'s internals.

Neither is clearly acceptable.

**Risk.** None for the claim: screen mode is the claim (02.7). Users of other hosted systems expect rootless mode, and its absence is the main usability gap.

**How to find out.** At P11:
1. Prototype the second way, since it changes no guest program.
2. Measure how often `devdraw.c`'s screen structures changed over the last three years of 9front.

If they changed rarely, ship it as experimental, with a check in `r9 9h verify` that fails when `devdraw.c` changes. If they changed often, record that rootless mode waits for a guest-side interface that 9front itself would accept.

**Deadline.** P11.

## 5. Plan 9 binaries of another architecture on a host

**Belief.** It is not claimed (02.7, 10.9).
- Rosetta translates only Mach-O and Linux ELF, so it cannot run amd64 9front on Apple Silicon.
- QEMU TCG runs either architecture anywhere, which is Q.

**Risk.** None for this goal. A user with amd64 Plan 9 binaries on an M-series Mac uses Q, or rebuilds for arm64, which is cheap because Plan 9 builds from source in minutes.

**How to find out.** Nothing to measure.

**Deadline.** None.

## 6. An arm64 Linux builder

**Belief.** `9h` arm64 on Linux uses KVM with a vGICv3 and the SMCCC filter (10.3, Linux 6.4). We believe it works as designed on an Ampere host. 04.4 also named a Raspberry Pi 5, but we believe its GIC-400 is a GICv2. KVM gives a guest a GICv3 only on a host with GICv3 hardware, so the Pi 5 cannot run `9h` arm64 without a GICv2 path that neither 9front's `qemu` kernel nor 10.3 has.

**Risk.** Without the builder, `9h` arm64 on Linux is not tested, and 02.7's Linux arm64 row stays out of the claim.

**How to find out.**
1. Confirm the Pi 5's interrupt controller from its device tree.
2. Get time on an Ampere host. The cloud kind is enough, if it exposes `/dev/kvm` with a GICv3.
3. Run `r9 9h boot` and the GUI scripts headless.

**Deadline.** P8, or 02.7 records the row as unclaimed.

## 7. Whether a weekly pin is right for `front`

**Belief.** 9front commits to `front` most days. A weekly pin, advanced only when G passes its baseline three times (04.1), is frequent enough to find rucc regressions against new code within a week. It is also slow enough that G's three baseline runs fit.

**Risk.**
- If `front` breaks G for weeks at a time, the pin is stale and the claim is about old code.
- If 9front changes headers or the compilers often, a weekly pin hides which change broke R.

**How to find out.** In P0:
1. Build G on one commit per week for the last year of `front`.
2. Count the weeks with no G-good commit.
3. Count the weeks with a change under `/sys/src/cmd/cc`, `6c`, `7c`, `6l` or `7l`.

If either count is high, change the cadence before P10 makes it routine.

**Deadline.** P10.

## 8. The kernel's SIMD rule

**Belief.** Plan 9's kernels save floating-point and SIMD state lazily, for user processes only. Kernel code must not touch SSE or NEON registers, and kencc never emits them for integer code. rucc might, for `memset` and `memmove` expansion and for vectorized loops. There are two candidate rules (09.3):
1. The shim sets `R9_KERNEL=1` for units under `/sys/src/9` and `/sys/src/boot`, and rucc uses its general-registers-only mode for them.
2. rucc never uses SIMD registers for integer code on Plan 9 targets.

We lean to the second. It matches kencc, it needs no path rule, and it costs only vectorization, which P9 can measure.

**Risk.** If the first rule is chosen and a kernel file lives outside those paths, the kernel's SIMD state is corrupted. The failure is intermittent and shows up far from its cause. `/sys/src/9` contains drivers and `/sys/src/libc` is linked into the kernel too.

**How to find out.**
1. At P5, `r9 objdiff` lists every SIMD instruction in R's kernel objects.
2. The rule is the one that leaves that list empty with the smaller loss on 13.2's library benchmarks.

**Deadline.** P5.

## 9. Whether 9front's arm kernels boot on QEMU `virt`

**Belief.** 9front's 32-bit arm kernels are for boards: the Raspberry Pi (`bcm`), Kirkwood (`kw`) and OMAP (`omap`). None targets QEMU's `virt` machine. The Pi kernel may boot under QEMU's `raspi2b` machine, which emulates enough of the Pi 2 for some systems.

**Risk.** If nothing boots, arm is built by rucc and checked by `r9 objdiff`, the fixed point and `tree-diff`, but no R kernel or user space ever runs. That is the status of arm64 boards already (09.1).

**How to find out.** At P10, before P11 starts:
1. Boot G's `bcm` kernel under `qemu-system-arm -M raspi2b`, to a shell on the serial console.
2. If that fails, count the work for a `virt` kernel. It would be new code in the tree, which this plan does not write.

**Deadline.** P11.

## 10. The 9P cost of a compile

**Belief.** rucc runs on the host and reads the guest's headers over 9P (05.4). A typical unit includes `u.h`, `libc.h` and 3 to 10 more headers. That is 30 to 80 round trips, which add 2 to 8 ms to a compile that `6c` does in about 5 ms. With `rucc-9p`'s `qid` cache, repeated headers cost one `walk` and a `stat` per build, not per unit. We believe that brings the 9P wait below 1 ms per unit.

**Risk.** QEMU's user network on macOS may be slower than on Linux, by enough to make R's build wall time over three times G's. That is the limit in 13.5. R's builds are then slow enough to cut the nightly's coverage.

**How to find out.** In P0, from a guest to a host process:
1. Time 10,000 `walk`, `open`, `read` and `clunk` sequences over the QEMU user network, on gpc and on the M4.
2. Repeat with a tap device on Linux and `vmnet` on macOS.

If the cost is too high, the alternative is to copy the guest's `/sys/include` and `/$objtype/include` once per build into a content-addressed host directory and serve rucc from that. That is still the guest's sysroot, read once.

**Deadline.** P1.

## 11. The GIC on HVF

**Belief.** macOS 15's `hv_gic_create` gives the guest a GICv3 with the same register layout as QEMU's `virt` machine (10.3). 9front's arm64 `gic.c` then programs it unchanged. The virtual timer's `VTIMER_ACTIVATED` exits are handled as the framework documents.

**Risk.** If the framework's GIC differs from what `gic.c` expects, the monitor emulates the distributor and redistributors itself. That is about 1,000 lines and two weeks in P7, with a cost in every interrupt. Possible differences:
- an unsupported register;
- different reset values;
- no support for the affinity routing 9front uses for secondary processors.

**How to find out.** In P7's first week, boot G's `qemu` kernel under a minimal monitor with `hv_gic_create`, to the first timer interrupt on every processor.

**Deadline.** P7.

## 12. Intel Macs

**Belief.** Intel Macs could run `9h` amd64 through the x86 Hypervisor.framework. The framework has no in-kernel APIC or I/O APIC there, so the monitor would emulate both, which KVM does for Linux. It is not planned and not claimed (02.6, 10.4). Apple no longer sells Intel Macs, and macOS 26 is the last release for them.

**Risk.** None for this goal.

**How to find out.** Nothing to measure. A contributor with an Intel Mac and the APIC work is welcome after P8.

**Deadline.** None.

## 13. Linux without KVM

**Belief.** `9h` requires a hypervisor. On Linux without `/dev/kvm`, as in some CI and nested environments, it exits with an error naming the device (10.9). Such a host runs Q under QEMU TCG with drawterm, which is not H.

**Risk.** None for the claim. It limits who can run `9h`. Without a TCG fallback, `9h` cannot be the one command a user runs everywhere.

**How to find out.** At P7, decide whether `9h` should start QEMU TCG with the same disk and present its screen through `h9-gui` as drawterm does. That would be a convenience mode, labelled as Q in every report.

**Deadline.** P7.

## 14. A virtual clock in `9h`

**Belief.** Most GUI programs are exact or masked (02.7, 12.6). The programs that are not depend on wall time: clocks, animations, games and `stats`. A virtual clock mode, in which the monitor advances guest time by a fixed step per scheduler tick, would make more of them exact. We believe the masks are enough to reach 60 of 80 without it.

**Risk.** If fewer than 60 of 80 programs are exact or masked, P8's exit criterion fails, and the virtual clock is needed. It costs about two weeks:
- the monitor's time hypercalls;
- the timer device;
- a check that `rio` and `libthread` behave with a stepped clock.

**How to find out.** At P8's midpoint, count the exact and masked scripts with real time on G/H. Build the clock only if the count is short.

**Deadline.** P8.

## 15. Reporting upstream

**Belief.** This plan finds bugs in other projects:
- in kencc and the tree (question 16), in the fixed point (question 2), and in G's builds;
- in zig's Plan 9 facts (15.5);
- in GCC 16's `-fplan9-extensions` (15.2).

A person reports each one, with a reproducer, to 9front's mailing list, zig's or GCC's tracker. The harness never files them.

**Risk.** None for the plan. A rucc project that finds 9front bugs and keeps them is seen badly, and fixes upstream make G better for everyone.

**How to find out.** Name the person responsible by P1. `r9 report --check-filing` lists findings older than 14 days that have no upstream link.

**Deadline.** P1.

## 16. When kencc is wrong

**Belief.** G is the oracle for what the tree's programs do, not for what C means. kencc has places where it is not C:
- unsigned-preserving conversions (06.8);
- the "unspecified" name resolution of 06.2;
- integer constant typing.

`-fkencc` copies kencc's behaviour on purpose, so G and R agree there. A kencc bug that the tree does not depend on is different: copying it is wrong, and not copying it can make a test differ.

**Risk.** A `diffrun` or `mk test` difference can then go two ways:
- it is a kencc bug, but it is filed as a rucc bug, and rucc is "fixed" to copy it;
- it is a rucc bug, but it is excused as a kencc bug.

Both make the claim weaker.

**How to find out.** Every R/G difference that rucc's developers believe is kencc's fault needs three things:
1. a reduced case in `rucc-compat/plan9/`;
2. the C standard's answer, with the clause;
3. GCC 16's answer with `-fplan9-extensions` on a host target.

If the tree's behaviour depends on kencc's answer, `-fkencc` copies it and 06 records it. If not, rucc keeps C's answer. The test then gets a mask in `exclusions.toml`, with the case number, and the bug is reported upstream (question 15).

**Deadline.** P3, when `diffrun` first runs on R.
