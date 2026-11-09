# Milestones

There are twelve milestones, P0 to P11. Each one gives:
- its scope;
- the work in each repository;
- an exit criterion that an `r9` command checks;
- a cost in engineer-weeks.

The order follows four decisions made earlier in this folder:
- **amd64 goes first.** It has KVM on every Linux builder, the performance gate, and the tree's most-used kernel (`pc64`).
- **User space goes before the kernel.** The kencc ABI and the `.6` writer are proven on `libc` and commands, where a bug crashes one program, before they are trusted with `pc64`.
- **The tree's assemblers stay until P6.** Plan 9 assembly is its own syntax and 440 files. The census counts those units as `ref-as` until rucc replaces `6a` and `7a` (07.5).
- **`9h` does not wait for rucc.** The `9h` kernel and monitor are tested with G's build first: G/H against G/Q. So P7 and P8 run in parallel with the compiler milestones, and R/H follows as soon as R's user space exists.

## Summary

| | Scope | Rows | Cost | Critical path through |
|---|---|---|---|---|
| P0 | the instrument: pins, guests, `r9-cc` recording, G baselines, census, demands, measurements | amd64, arm64; G only | 4 | the builder image and the G fixed point |
| P1 | rucc speaks Plan 9: targets, `--driver=kencc`, the dialect's front half, the preprocessor, pragmas, 9P sysroot, `-fplan9-extensions` | amd64 `-fsyntax-only` over the tree | 8 | `rucc-9p` and the shim round trip |
| P2 | the amd64 ABI and `.6` writer: convention, layout, `extern register`, conversions, helpers | amd64 libraries and the first commands, under bring-up | 12 | the `Prog` layer and `optab` conformance |
| P3 | the amd64 world: signatures, `varargck`, acid; `ref-c` = 0 for `mk install` | amd64 user space on a G kernel | 6 | signatures bit-exact with kencc |
| P4 | arm64: `.7` writer, arm64 convention, `7l` `optab` | arm64 user space on a G kernel | 6 | `.7` records and TCG test time |
| P5 | the kernels: `pc64` and arm64 `qemu` built by R boot and pass; the first graded runs | amd64, arm64 `current`, graded | 4 | the kernel's layout and register assumptions |
| P6 | the assembler: `6a` and `7a` syntax, `ref-as` = 0 | amd64, arm64 | 5 | byte identity on 440 files |
| P7 | `9h` core: kernel files, monitor on HVF and KVM, devices, headless | G/H and R/H on the M4 and gpc | 8 | the GIC and timer on HVF |
| P8 | the GUI: host back ends, 80 scripts, hostchecks | Q and H, both hosts | 10 | writing and stabilizing 80 scripts |
| P9 | performance: the runtime and compile-time gates | amd64 on server3 | 6 | whatever the profiles show |
| P10 | every version, steady state, and 386 | `current` and `releases`; 386 | 6 | the `.8` writer and `8c` convention |
| P11 | arm, and the rootless decision | arm on QEMU `virt`; open question 4 | 7 | whether 9front's arm kernels boot on `virt` |

The total is 82 engineer-weeks before contingency, and about 91 with it (16.14). That is about 1.75 engineer-years. Of it, 28 to 38 weeks are the rucc work 03.19 can already see. The rest is:
- the harness;
- `9h`;
- the GUI scripts;
- the code generation bugs that only a 2.3-million-line build will find.

## 16.1 P0: the instrument

**Scope.** Nothing is built by rucc. At the end of P0, the harness can do the following for amd64 and arm64:
- fetch the tree;
- make the builder image;
- build G three times;
- boot G;
- run `mk test`, the smoke suite, `diffrun` and the fixed point;
- state what the builds demand from a compiler.

**`rucc-plan9`.**
- The workspace of 14.2 on `rucc-harness`.
- `r9 fetch`, `r9 sets --write` and `r9 check`.
- `r9 image build` and the `g-base` images.
- `r9-cc` in recording mode, `r9-build` and `r9-boot`.
- The smoke suite, `r9 baseline`, `r9 census` and `r9 demands`.

**rucc.** The issues of 15.1 opened, each with its milestone. The two spec edits of 15.1 proposed.

**Measurements P0 owes the rest of the plan.**
- **G build and test times** per row, replacing 04.5's estimates.
- **`6c`'s compile time** per unit, for open question 1.
- **9P round-trip costs** from a guest to a host process over QEMU's user network, on Linux and macOS (05.4, open question 10).
- **Libraries for other architectures.** What `mk kernels` needs from them, and whether `objtype=$o mk libs` is required (09.1).
- **The fixed point of G.** Is kencc built by kencc built by kencc byte-identical to G? That is, does the fixed point hold for G itself (04.2)? If it does not, 12.4 is redesigned before P3.
- **Where each release came from.** The tree hashes of 11952 and 11554 mapped to commits (01.1).

**Exit criterion.**
1. `r9 sets --write` produces `pins.toml` with no hand edits.
2. `r9 baseline --objtype=amd64,arm64` completes three G runs of every suite of 12. Every graded G result is stable, and every flaky one is listed.
3. A fourth G build is byte-identical to the first (`r9 tree-diff`, `r9 objdiff --tree`).
4. `demands.toml` exists for both architectures, and its ranked list is attached to the issues of 15.1.

**Cost.** 4 weeks.

## 16.2 P1: rucc speaks Plan 9

**Scope.**
- rucc accepts every amd64 unit of the tree with `-fsyntax-only`.
- The compile path from the guest to the host works end to end.
- `-fplan9-extensions` works on host targets.

**rucc.** These are the P1 rows of 15.1:
- targets;
- `--driver=kencc`;
- the dialect's front half: keywords, members, `USED`, `SET`, `signof` and `typestr` parsing, lax declarations;
- the kencc preprocessor and pragmas;
- the lexer;
- `rucc-9p`;
- `-fplan9-extensions`;
- the brace-elision fix.

**`rucc-plan9`.**
- `r9 ccd`.
- `r9-cc` in forwarding mode, with `R9_BRINGUP=cc:auto`.
- `r9 flags-diff`.

**Corpora.** `rucc-compat`: `plan9/const`, `plan9/cpp` and `gcc-plan9-ext`. `rucc-corpus`: `kencc-members` and `brace-elision`.

**Exit criterion.**
1. `r9 demands --objtype=amd64` reports no unit whose first error is a front-end error.
2. An R build with `R9_BRINGUP=cc:auto` completes `mk install`. Every unit reaches rucc and is delegated only for code generation, which does not exist yet.
3. `rucc-compat/gcc-plan9-ext` passes against GCC 16.

**Cost.** 8 weeks.

## 16.3 P2: the amd64 ABI and the `.6` writer

**Scope.** rucc emits `.6` objects that `6l` links with G's, for libraries and commands.

**rucc.** These are the P2 rows of 15.1:
- `Convention::Kencc` and `Layout::Kencc` for amd64;
- `extern register`;
- the conversions;
- lax types;
- the runtime helpers;
- the `.6` writer and the `Prog` layer;
- `optab` conformance.

**`rucc-plan9`.**
- `r9 optab`, `r9 objcheck`, `r9 objdiff` and `r9 sdiff`.
- `r9 mix`.
- `r9 helpers`.

**Corpora.**
- `rucc-compat`: `plan9/abi` and `plan9/layout`.
- `rucc-corpus`: `kencc-conversions`, `kencc-layout` and `kencc-regs`.

**Exit criterion.**
1. `plan9/abi` and `plan9/layout` pass in the guest.
2. With bring-up, R builds `libc`, `libmp`, `libsec`, `libdraw`, `libmemdraw`, `libthread`, `libbio`, `rc`, `mk` and `awk`. `mk test` in `libc`, `libmp`, `libsec` and `libmemdraw` passes with R's libraries on a G kernel.
3. `r9 objcheck` passes on every object rucc wrote.
4. The delegated-unit count is under 5% of amd64 units.

**Cost.** 12 weeks. This is the largest milestone and the riskiest: a new convention, a new object format, and a linker that rejects anything outside its table.

## 16.4 P3: the amd64 world

**Scope.** All of amd64 user space is built by rucc.

**rucc.** These are the P3 rows of 15.1:
- signatures;
- `varargck`;
- acid output.

**`rucc-plan9`.**
- `r9 diffrun`.
- `r9 fixedpoint`.
- `r9 warn-diff`.
- `r9 2x2` for user space.

**Corpora.** `rucc-compat/plan9/signof`, `rucc-corpus/kencc-varargck`.

**Exit criterion.**
1. `mk install` on amd64 has `ref-c` = 0, with no bring-up.
2. `r9 tree-diff` is clean.
3. R's user space on G's kernel passes the following:
   - `mk test`;
   - the smoke suite;
   - `diffrun`'s 40 programs;
   - the fixed point for the user-space tools (12.4).

**Cost.** 6 weeks.

## 16.5 P4: arm64

**Scope.** P2 and P3 again for arm64.

**rucc.**
- `Convention::Kencc` for arm64.
- The `.7` writer.
- `7l` `optab` conformance.

**`rucc-plan9`.** arm64 rows in every command. Builds run on the M4 under HVF, and on gpc under TCG.

**Exit criterion.** The P3 criteria on arm64.

**Cost.** 6 weeks.

## 16.6 P5: the kernels

**Scope.** R builds and boots the `pc64` and arm64 `qemu` kernels. The rest of `mk kernels` is built: by rucc for arm64 boards, and as `other-arch` for the rest (09.1). The amd64 half can start right after P3, in parallel with P4.

**rucc.** Kernel mode (09.3, open question 8).

**Exit criterion.**
1. `mk install` and `mk kernels` have `ref-c` = 0 over `amd64` and `arm64` (02.2).
2. `r9 boot` passes R kernel with R user space on both architectures, and the 2x2 of 12.7 passes on every suite.
3. **The first graded run.** It is the `current` set on amd64 and arm64. The census line says `ref-as` is non-zero until P6.

**Cost.** 4 weeks.

## 16.7 P6: the assembler

**Scope.** rucc replaces `6a` and `7a`.

**rucc.** `rucc-asm/src/plan9/`.

**`rucc-plan9`.** `r9-cc` is also bound over `6a` and `7a`.

**Corpora.** `rucc-compat/plan9/asm`.

**Exit criterion.**
1. All 440 `.s` files assemble byte-identical to `6a`/`7a` output (`r9 objdiff`).
2. The census shows `ref-as` = 0.
3. P5's graded run passes again with the assembler in the slot.

**Cost.** 5 weeks.

## 16.8 P7: `9h` core

**Scope.** The `9h` kernel and monitor run 9front headless on both hosts. It starts with G's build, and R's once P3 or P4 lands.

**`rucc-plan9`.**
- `9h/arm64`, `9h/amd64`.
- `h9-vm`, `h9-hvf`, `h9-kvm`, `h9-dev`, `h9-9p`.
- The `9h` binary with `--headless` and `--script`.
- `r9 9h verify|boot|smoke`.

**Exit criterion.**
1. `r9 9h verify` passes.
2. G/H boots to `rio` (headless) and passes the smoke suite on the M4 (arm64, HVF) and on gpc (amd64, KVM).
3. The same holds for R/H as soon as R's user space exists.
4. `r9 9h boot --strict` has no unknown exit.

**Cost.** 8 weeks.

## 16.9 P8: the GUI

**Scope.**
- The host back ends of 11.
- The 80 GUI scripts.
- The host checks.

**`rucc-plan9`.**
- `h9-gui`.
- `gui/*.r9`, with ROMs and data.
- `r9 gui` and `r9 hostcheck`.

**Exit criterion.**
1. Every script passes G/Q against G/H. This checks `9h` itself.
2. R/H passes against G/Q for all 80 programs, with at least 60 exact or masked (02.7), on the M4 and on gpc.
3. `r9 hostcheck` passes on Cocoa, X11 and Wayland.

**Cost.** 10 weeks. About six of them are the scripts: 80 scripts at two or three a day, each needing G's three stable runs and its masks.

## 16.10 P9: performance

**Scope.** The gates of 02.5.

**rucc.** Whatever the profiles show (13.3). Expected:
- spill and reload around calls under the all-caller-saved convention;
- inlining limits for the size gate.

**`rucc-plan9`.** `r9 bench`, `r9 speed`, and `r9 mix bisect --perf`.

**Exit criterion.**
1. The runtime gates of 02.5 pass on server3.
2. `r9 speed --gcc` is at most 0.5.
3. The kencc ratio is published, and open question 1 is decided.

**Cost.** 6 weeks.

## 16.11 P10: every version, steady state, and 386

**Scope.**
- **Versions.** The `releases` set (11952 and 11554) joins the graded runs, and the pin advances weekly (02.4).
- **386.** The `.8` writer and `8c`'s convention, which is the stack-only 32-bit convention. The `pc` kernel, 9boot and `boot/efi`'s 32-bit half move from `other-arch` to rucc.

**Exit criterion.**
1. Every member of both sets has a graded run.
2. 386 has `ref-c` = 0, and its `pc` kernel boots under QEMU/KVM.

**Cost.** 6 weeks.

## 16.12 P11: arm, and the rootless decision

**Scope.**
- **arm.** The `.5` writer and `5c`'s convention. Whether any 9front arm kernel boots on QEMU `virt` is open question 9. If none does, arm is built but not booted, like the boards.
- **Rootless.** Open question 4 is decided, either with a prototype of 11.7's first option or by recording that it will not be done.

**Exit criterion.**
1. arm has `ref-c` = 0.
2. The question has an answer, recorded in 17.

**Cost.** 7 weeks.

## 16.13 Parallel tracks

With two engineers:
```
Engineer A (compiler):  P0 ── P1 ── P2 ── P3 ── P4 ── P5 ── P9 ── P11 (arm)
Engineer B (runtime):   P0 ── P7 ── P8 ── P6 ── P10 ── P11 (rootless)
```

- **P7 and P8 need nothing from rucc.** They are graded first with G/H, then with R/H as soon as P3 (amd64) or P4 (arm64) lands.
- **P6 needs P2's `Prog` layer.** That lands before B finishes P8.
- **P10's 386 work** is compiler work on the `Prog` layer, and B takes it to keep A on the critical path.

**Dates, if work starts on 16 November 2026.** These dates assume nothing slips:
- A finishes P3 in late June 2027.
- The first graded run (P5) is in September 2027.
- The performance gates (P9) are met in late October 2027.
- The plan ends around December 2027: about 13 months with two engineers, or about 15 with contingency.

9front will tag two or three releases in that time, and P10 takes them in.

## 16.14 Cost table

| | Weeks | Cumulative | Uncertainty |
|---|---|---|---|
| P0 | 4 | 4 | low |
| P1 | 8 | 12 | medium (9P on macOS, the preprocessor's corners) |
| P2 | 12 | 24 | **high** (new convention, object format and `optab`) |
| P3 | 6 | 30 | **high** (first full world; signatures bit-exact) |
| P4 | 6 | 36 | medium |
| P5 | 4 | 40 | medium |
| P6 | 5 | 45 | low to medium |
| P7 | 8 | 53 | **high** (HVF's GIC and timer; KVM amd64 boot) |
| P8 | 10 | 63 | **high** (script stability) |
| P9 | 6 | 69 | medium |
| P10 | 6 | 75 | medium |
| P11 | 7 | 82 | medium (open question 9) |

With 25% contingency on P2, P3, P7 and P8, which total 36 weeks, the contingency is 9 weeks. That brings the plan to about 91 weeks.

The smallest piece of this plan that stands on its own as a claim is P0 to P5 without P4's arm64: 34 weeks. That claim is amd64 9front, user space and kernel, built entirely by rucc apart from its assembly, booting and passing its tests. The smallest GUI claim is P7 and P8 with G's build: 18 weeks, with no rucc work at all. It is worth making early, because it is useful to people whatever happens to the compiler.
