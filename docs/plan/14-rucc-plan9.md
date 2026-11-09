# The `tamnd/rucc-plan9` repository

`rucc-plan9` does the following:
- pins 9front trees and release images;
- builds G and R in 9front guests, with rucc in the compiler slot;
- writes disk images, boots them, and runs the tests of 12 and the benchmarks of 13;
- compares R with G, and reports.

It also contains `9h` (10, 11): the paravirtual arch files and the host monitor. It has the shape of `rucc-bsd` and `rucc-kernel`. Whoever knows `rb` or `rk` should find their way around `r9`.

It is the fourth harness in the family. It depends on `rucc-harness` (15.4) for pins, runs, baselines, reports and machine provisioning, as `rucc-bsd` does.

## 14.1 What it may and may not contain

These are `rucc-bsd`'s four rules, stated for Plan 9:
- **No compiler code.** Nothing here compiles C for a graded build. Every compile goes to rucc or to G's kencc through `r9-cc`, which records the call and changes nothing. A fix to rucc goes to `tamnd/rucc`.
- **No patches and no copied 9front files.** Trees are fetched by commit hash from 9front's git server or its GitHub mirror, and release images by SHA-256. The harness never writes into a tree. A build writes into the guest overlay, which is thrown away (05.2), and the namespace of `r9-build` is the only thing the harness changes.
- **No reductions.** A reduced reproducer goes to `rucc-corpus`, or to `rucc-compat` if it compares two compilers. `rucc-plan9` links to it by issue number.
- **No hidden configuration.** Everything that influences a graded build is a committed file:
  - the pins and the image hashes;
  - the guest scripts;
  - the kernel configurations used (the tree's own names, plus `9h`'s);
  - the GUI scripts and their masks;
  - the exclusions.

Two rules are new:
- **`9h`'s kernel files are new code.** `9h/arm64` and `9h/amd64` hold only files whose names appear nowhere in the tree's `port/` or arch directories (10.2). A file derived from a tree file says which, at the top. `r9 9h verify` checks both rules. The files are MIT licensed, as 9front is.
- **Guest programs are rig, not claim.** `r9-cc`, the GUI script helper, the test probes and `r9-bench` are Plan 9 C programs in `guest/`. They are built by G for the rig, except `r9-bench`, which is built by the build under test (13.2). None of them is counted in the census or the GUI claim.

## 14.2 Layout

```
rucc-plan9/
  Cargo.toml                   workspace; depends on rucc-harness (15.4) and rucc-9p (15.1)
  crates/
    r9/                        the CLI
    r9-pins/                   pins.toml, sets.toml, image hashes, fetch and verify (04)
    r9-ccd/                    the host side of the compiler slot (05.4)
    r9-image/                  qcow2 overlays, gefs root building, image sync over 9P (05.2)
    r9-qemu/                   machines, consoles, timeouts (09.4)
    r9-census/                 compile.jsonl, census, demands, flags-diff (05.6)
    r9-obj/                    .6/.7 reader: objdiff, objcheck, sdiff, optab extraction (07)
    r9-acid/                   acid output parsing for layout comparison (07.7)
    r9-mix/                    mixed builds and bisection (12.8)
    r9-gui/                    script language, snap comparison, masks (12.6)
    r9-tests/                  mk test, smoke, diffrun parsing (12)
    r9-bench/                  bench and speed (13)
  guest/
    r9-cc/                     the compiler-slot shim, Plan 9 C (05.3)
    r9-build                   the rc build script (05.3)
    r9-boot                    the rc boot-and-test script (09.4)
    smoke/                     the smoke suite (09.5)
    gui-helper/                drives /dev/wsys, mousein, kbdin for scripts (12.6)
    probe/                     the hostcheck probe (11.9)
    bench/                     r9-bench, Plan 9 C, built by the build under test (13.2)
  9h/
    arm64/                     the 9h kernel's new files for arm64 (10.2)
    amd64/                     the same for amd64
    monitor/                   the host monitor, Rust (10.8)
      h9-vm/ h9-hvf/ h9-kvm/ h9-dev/ h9-9p/ h9-gui/ 9h/
  gui/
    <prog>.r9                  one script per GUI program, 80 in the claim (02.7)
    roms/                      homebrew ROMs and their LICENSES (12.6)
    data/                      fonts, documents and images the scripts open
  diffrun/
    cases/                     one rc script per case (12.5)
    inputs/                    free inputs, or fetch-by-hash manifests for large ones
  pins.toml                    trees and images per set (04.1)
  sets.toml                    set rules; r9 sets expands them into pins.toml
  rows.toml                    objtype, QEMU machine, kernel config, console, timeouts
  machines.toml                gpc, server1-3, the M4: cores, disk, hypervisor (04.4)
  exclusions.toml              tests excluded, each with G's failure log
  demands.toml                 the feature census (05.6)
  baselines/<pin>/<objtype>/   G results, three runs (12.1)
  runs/<id>/                   one directory per run (14.6)
  reports/
    nightly.md
    weekly.md
  .github/workflows/
    pr.yml                     harness and monitor tests only, no tree builds
```

## 14.3 Commands

| Command | Does |
|---|---|
| `r9 fetch` | fetches the pinned trees into bare git repositories and the images by SHA-256; checks every hash |
| `r9 sets` | maps release images to commits by tree hash (01.1) and regenerates `pins.toml` |
| `r9 check` | validates every `.toml` against its schema and `pins.toml` against `r9 sets` |
| `r9 image build\|sync\|boot` | makes `g-base`, writes a tree into a guest over 9P from git objects, makes bootable images (05.2, 09.4) |
| `r9 ccd` | the host daemon that runs rucc for `r9-cc` (05.4) |
| `r9 build --side=R\|G` | boots a builder guest, runs `r9-build`, collects the outputs |
| `r9 census` | origins of every unit, what never reached the slot, `--gui` for the GUI census (05.6, 02.7) |
| `r9 demands` | replays G's units through `rucc -fsyntax-only` and ranks the first errors (05.6) |
| `r9 flags-diff` | the argv of every unit, R against G (05.5) |
| `r9 tree-diff` | the set of output files, R against G (05.5) |
| `r9 warn-diff` | warnings lost and added (05.8) |
| `r9 objdiff` | record-level diff of `.6`/`.7` files; `--tree` over a whole build (07.6, 12.4) |
| `r9 objcheck` | checks a rucc object against the format and the linker's `optab` (07.6) |
| `r9 optab` | extracts `optab_{amd64,arm64}.rs` from the pin's `6l`/`7l` (07.3) |
| `r9 sdiff` | R and G code side by side, per function (12.8) |
| `r9 helpers` | the compiler-generated calls in G's objects (08.6) |
| `r9 mix` | builds any R/G mix; `bisect` finds the unit, `--perf` times instead of testing (12.8, 13.3) |
| `r9 boot` | boots R and G images and runs the smoke suite (09.4, 12.3) |
| `r9 test` | `mk test` in a guest, parsed (12.2) |
| `r9 fixedpoint` | rebuilds the tree with R's kencc and compares with G (12.4) |
| `r9 diffrun` | the 40 deterministic programs (12.5) |
| `r9 gui run\|compare` | GUI scripts and pixel comparison (12.6) |
| `r9 2x2` | a test on the four kernel and user-space combinations (12.7) |
| `r9 9h verify\|boot\|smoke` | the `9h` checks (10.10) |
| `r9 hostcheck` | the host checks on a real desktop (11.9) |
| `r9 bench` | the 40 benchmarks on server3 (13.2) |
| `r9 speed` | compile time against GCC 16 and kencc (13.5) |
| `r9 baseline` | G's three runs per pin (12.1) |
| `r9 xcheck` | G's installed binaries against the release image's (04.2) |
| `r9 report` | writes the nightly and weekly reports; `--check-filing` audits issues (14.7) |

`9h` itself is a separate binary built from `9h/monitor`. `r9` drives it but does not contain it, so a user who only wants to run Plan 9 programs installs `9h` alone.

## 14.4 Bring-up

Before rucc compiles the whole tree, a build would fail at the first unit rucc cannot handle. Bring-up lets rucc's coverage grow one unit at a time:
- `R9_BRINGUP=cc:<file>` names a list of units, by path, that `r9-cc` sends to G's kencc instead of rucc. They are recorded with origin `ref-c`.
- `R9_BRINGUP=cc:auto` sends any unit rucc rejects with a front-end error to kencc, and records it. A unit rucc accepts but miscompiles is not caught by this, and that is deliberate: auto mode only decides what rucc can parse.
- A run with any bring-up is ungraded. Its report says so on the first line (02.8), and its census lists the delegated units.

Bring-up is how P1 to P3 measure progress: the count of delegated units, by directory, falls to zero. When it reaches zero for a row, the row's graded run begins.

## 14.5 Machines and builders

| Machine | Role in `rucc-plan9` |
|---|---|
| gpc | G and R builds for amd64 (KVM) and arm64 (TCG); nightly; `9h` amd64 headless and X11/Wayland hostchecks |
| server1, server2 | `diffrun`, `mk test` fan-out, the GCC compile comparison host |
| server3 | benchmarks only (13.3) |
| the M4 | arm64 builds under QEMU/HVF; `9h` arm64 GUI suite; Cocoa hostcheck |

The builder images are 9front guests. `r9 image build` makes them from the release image and the pin's tree (05.2). They are kept per pin and `objtype`, about 2 GB each as qcow2 overlays.

## 14.6 Storage

Per run, `runs/<id>/` keeps:
- the manifest;
- `compile.jsonl` and its census;
- the test results;
- the console logs;
- the diffs;
- the GUI snaps of failing steps.

Images are kept for failures only, for 14 days. A 9front root is about 1.5 GB, so this is small next to the BSD harness. Baselines are kept forever, including every G GUI snap per pin, at about 200 MB per pin.

## 14.7 Reports

`reports/nightly.md` has this layout:
1. **Headline.** Per row (`amd64`, `arm64`) and per set: built, booted, tested, GUI, each green, red, or not run. Its first line is the census: the `ref-c` count, `ref-as` count, `other-arch` count, and whether any bring-up was used.
2. **Regressions** since the last night, each with its bisected rucc commit.
3. **Differentials.** `flags-diff`, `tree-diff`, `objcheck`, `warn-diff`: counts and links.
4. **Tests.** `mk test`, smoke, the fixed point, `diffrun`, and GUI by class (exact, masked, smoke) against 02.7's counts.
5. **`9h`.** `9h verify`, boots per host, and hostchecks.
6. **Performance.** Text sizes nightly, and benchmarks when server3 ran them.
7. **Compile time and memory.** The ratio against GCC 16, the ratio against kencc, and the 10 slowest units.
8. **Demands.** Missing features by blocked units, until zero.
9. **Open findings**, with issue links.

`reports/weekly.md` adds the `releases` set, the benchmark table, and a summary of the week's rucc commits by milestone.
