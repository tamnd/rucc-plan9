# Tests and oracles

The oracle is always G: the same tree, built by kencc, run the same way. R passes a test when it gives what G gives. No expected output is written by hand, except for the smoke suite (09.5) and the host checks (11.9), which test the rig as much as the build.

## 12.1 The oracle and flakiness

Each test runs three times on G at each pin. A test is **graded** when G gives the same result all three times. A test whose G result varies is **flaky**. It is reported with its variation, and it does not grade R. A flaky test that becomes stable is graded again from the next pin. `r9 baseline` stores G's results per pin under `baselines/<pin>/<objtype>/<suite>.json` with the hash of each run.

R runs once. A graded test that fails on R runs twice more. If it fails all three times, it is a **failure**. If it fails once or twice, it is an **R-flake**. R-flakes are reported, filed with the `flaky` label, and counted. They are not failures, but more than three R-flakes in one suite, in one run, fail the run. A miscompile can look like a flake when it depends on timing, and that rule stops flakes from hiding one.

## 12.2 `mk test`

The tree's tests (01.4) run in a Q guest booted from the build's image:
```rc
cd /sys/src && mk test >[2=1] | tee /tmp/r9/test.log
```
There are 13 directories and 69 programs and scripts. `r9 test parse` turns the log into one result per test. The tree's `mktest` prints a pass or fail line for each test it runs. Where a directory's test prints only its own output, the result is that output's hash. R must pass every graded test G passes.

The suite is small, and it mostly tests libraries. Its value is in `libmp`, `libsec` and `libmemdraw`. These are three libraries whose bugs are silent: a wrong carry, a wrong digest, or a pixel off by one.

## 12.3 Boot

The boot test is 09.4's. Its pass conditions are:
- **The console matches.** The serial log matches G's line by line after `r9 boot normalize`, which removes times, addresses, `pid`s and the memory size line.
- **The smoke suite passes** (09.5).
- **On amd64, `rio`'s first screen matches G's.** The screen is read from `/dev/screen` and compared with the clock masked.

The default timeouts (09.4) are replaced by three times G's median once P0 has measured it. A timeout is a failure, never a flake, because a kernel that hangs on boot one time in three is broken.

## 12.4 The compiler fixed point

R builds the tree's own compilers and linkers: `6c`, `6l`, `6a`, `7c`, `7l`, `7a`, and the shared `cc` library. The compilers, assemblers and linkers for both architectures are about 54,000 lines of dense C, and `6c` alone (`cc/` and `6c/`) is about 21,700. They process every line of the tree. A rucc miscompile in it shows up as a different object somewhere in 2.3 million lines of output, which makes this the strongest single test in the folder.

The test:
1. Take R's image, which contains R's `6c`, `6l` and `6a`.
2. Build the whole tree again in it, with no shim. This is "Rk", kencc built by rucc.
3. Compare every `.6` and every installed binary with G's, byte for byte (`r9 objdiff --tree`).

kencc's output depends only on its input, and G is kencc built by kencc. So Rk equals G unless rucc miscompiled kencc. A difference names a unit whose object differs. `r9 objdiff` then shows the first differing record, which usually points at the kencc function R got wrong. `r9 mix` (12.8) bisects kencc's own objects to find which.

Both builds run `mk clean install`, but `mk clean` does not remove each output. An archive such as `libplot.a6`, a generated file such as `enam.c` and an installed rc script stay in R's image, and mk does not make them again. A link in Rk then uses objects that rucc compiled, and the comparison does not see the files that mk did not write. In the first runs, this gave 67 files that only G wrote and a different `plot` on both rows. Thus a build with the target `clean` first reads the list of changed files that the earlier build left in `/tmp/r9/changed`. It removes each file in the list that is not in `/$objtype/bin` or `/$objtype/lib`, and it gives the date 0 to each installed file. mk then makes each of them again, as in a new tree. `mk install` also writes to `/sys/lib`, `/rc/bin` and `/acme/bin`, which the list does not cover. The build also gives the date 0 to each file in these directories that is newer than the start of the earlier build, in `/tmp/r9/t0`. Without this, the second run on amd64 gave 3 files that only G wrote, because mk took `/sys/lib/tmac/tmac.pictures`, `/rc/bin/printfont` and `/rc/bin/postscript/psencoding` as up to date and did not make the files in `/sys/src` that it copies.

On arm64, the same test uses `7c`/`7l`/`7a` from P4. The fixed point is a full build, but it runs natively in the guest, so it is as fast as G's build (04.5).

## 12.5 `diffrun`

`diffrun` runs deterministic programs on fixed inputs under R's user space and G's, on the same G kernel, and compares the outputs byte for byte. The inputs are in `rucc-plan9/diffrun/inputs/`, and every one is free to redistribute. Each case is a short `rc` script.

There are 40 programs:

| Group | Programs | Inputs |
|---|---|---|
| Typesetting (9) | `troff`, `tbl`, `eqn`, `pic`, `grap`, `tr2post`, `tcs`, `gs`, `cpp` | the tree's own manual pages and `/sys/doc` papers, rendered to PostScript and then by `gs` to Plan 9 images; `tcs` over every charset |
| Text tools (10) | `awk`, `sed`, `grep`, `sort`, `uniq`, `diff`, `ed`, `yacc`, `lex`, `mk -n` | the tree's sources; `yacc` and `lex` on the tree's grammars; `mk -n` in every directory of `/sys/src` |
| Arithmetic (4) | `hoc`, `bc`, `dc`, `factor` | generated expressions, to 10,000 digits for `bc` and `dc` |
| Compression (5) | `gzip`, `gunzip`, `bzip2`, `bunzip2`, `tar` | a 200 MB corpus: the tree, a Wikipedia dump slice, and random bytes |
| Hashes and ciphers (4) | `sha1sum` (with `-2 256` and `-2 512`), `md5sum`, `auth/aescbc -d`, `git/fs` with `git/query` | the corpus; a fixed `aescbc` file; a mirror of the 9front repository, resolving 1,000 revisions and reading every blob |
| Images (4) | `jpg`, `png`, `topng`, `resample` | the JPEG and PNG conformance suites, converted to Plan 9 images and back |
| Audio (4) | `audio/flacdec`, `audio/oggdec`, `audio/mp3dec`, `audio/flacenc` | free test vectors, decoded to raw PCM |

Two rules keep the set honest:
- **A case is only in `diffrun` if G is deterministic on it.** G must agree with itself in all three runs of 12.1.
- **The programs are chosen for what they exercise.** They cover:
  - bignum arithmetic: `bc`, `dc` and `libmp` through `factor`;
  - floating point: `troff`'s arithmetic, `gs`, and the audio decoders;
  - bit-twiddling: the compressors and digests;
  - pointer-heavy code: `awk`, `yacc` and `git/fs`;
  - large switch and table code: `troff` and `lex`.

`diffrun` runs on server1 and server2 in about 15 minutes (04.5). A difference is reduced the same way as a fixed-point difference. `r9 mix` finds the unit, and `r9 sdiff` shows the code for the function.

## 12.6 The pixel oracle

GUI programs are tested by their pixels, and the pixels are exact because:
- **`memdraw` is software.** Every pixel in a Plan 9 window is computed by `libmemdraw` in the kernel's `devdraw`. No GPU and no host renderer touches it.
- **`rio` exports each window's image.** `/dev/wsys/N/window` reads back as a Plan 9 image file: a header and the pixels, in the window's channel format.
- **Input is scriptable.** `/dev/mousein` accepts mouse events and `/dev/kbdin` accepts key events, through `kbdfs`, as if they came from the hardware.

**Scripts.** Each GUI program has `rucc-plan9/gui/<prog>.r9`. A script is a sequence of steps:
```
# gui/acme.r9
window 1024x768                       # rio window size, fixed
run acme -f /lib/font/bit/lucsans/euro.8.font /usr/glenda/r9/gui/acme/demo.c
quiet                                 # wait until the image is still for 500 ms
snap open                             # record the window image as step "open"
mouse 3 at 200,40                     # button 3 click: look for the word
quiet
snap look
keys "hello, world\n"
mouse chord 1+2 at 120,300            # cut
quiet
snap cut
mask none
exit Del                              # the program's quit, then check it exits 0
```

`r9 gui run` executes a script against a running session. It drives `rio`'s files from a helper process in the guest, and collects each `snap` as an image file plus its hash. `r9 gui compare` compares R's snaps with G's step by step. It applies the masks, which are rectangles in window coordinates declared by `mask` steps. Its output is a pass, or the first differing step with a picture of the difference: G, R, and the XOR.

**Status.** On 9h, the input comes from the host and not from a helper process in the guest. A probe on the arm64 9h guest showed that text written to `/dev/kbdin` goes to the console window and not to the current window, and that the `/dev/kbdtap` of `rio` drops a write when the window is not the current one. The keys of the 9h console device and the 9h mouse go to the current window, as the keys and the mouse of a person do. Thus `r9 gui run` gives the steps of a script to the monitor on its control socket. The monitor has the steps `quiet` and `snap` for this (10). `quiet` watches the rectangle of the window under test, and `snap` writes that rectangle as a PNG file. `r9 gui` reads the pixels from the PNG file and records their SHA-256 hash.

**How `r9 gui run` drives the session.** The screen of 9h is 1024x768. The window under test goes in the bottom right corner, so that it does not cover the console window at (100,400). The `run` step types one line in the console. The line writes the command and `echo R9-GUI^-EXIT $status > '#c/cons'` to `/tmp/r9gui.rc`, and then runs `window -r x0 y0 x1 y1 rc /tmp/r9gui.rc`. A point in a script is in window coordinates, and `r9 gui` adds the corner of the window. The `exit` step gives the quit key of the program. When the program stops, the run file writes `R9-GUI-EXIT` and the status to the kernel console. The caret keeps the typed line from having this text. The `wait R9-GUI-EXIT` step of the control socket waits up to 60 seconds for it, and then `r9 gui` waits 1 second and clicks the console window. Rio drops the keys that come while the window of the program closes, so a console command after the exit key can lose keys. `r9 gui` reads the status from the last `R9-GUI-EXIT` line of `console.log`, and the status must be empty. The steps start at the first shell prompt of the console. `quiet` cannot see when rio is ready, because the `stats` window of the session draws each second, so a step before the prompt can go to the wrong window. `r9 gui` gives each key on its own `send` line, and the monitor waits until the guest reads it. A burst of keys lost keys in rio and in samterm. Before the exit key, the `absent R9-GUI-EXIT` step of the control socket makes sure that the program did not stop too early. `r9 gui run` writes `gui.json` with the hash of each snap. `r9 gui compare` writes the images `want.png`, `got.png` and `xor.png` of the first step that differs.

**Classes** (02.7):
- **exact**, with no masks. There is no allowance: the bytes are equal or they are not.
- **masked**, with masks covering less than half the window.
- **smoke.** The program draws, takes the script's input, and exits on its quit key with status 0, with no fault and no "suicide" on the console.

**Where scripts run.**
- **amd64 Q:** both R/Q and G/Q run in QEMU/KVM on gpc, under `rio` at a fixed screen of 1280×1024.
- **arm64 Q:** the `qemu` kernel has no draw device (09.4). arm64 scripts run in two ways:
  - G/Q through drawterm, as the reference;
  - R/H and G/H through `9h`.
  
  `9h`'s G/H result must equal the drawterm G/Q result. That checks `9h` itself, and then R/H is compared with G/H.
- **H on each host:** `9h run --headless --script` on the M4 (arm64) and gpc (amd64). The screen size is fixed, so a run is independent of the host's display.

**Fonts.** Scripts name their fonts explicitly. The default font comes from the environment, and a host could change the environment.

**Determinism traps** (each found by G's three runs, 12.1):
- **The clock and dates.** `clock`, `stats`, `date` in `acme`'s tag, and `faces` timestamps. These are masked.
- **Random seeds.** `mines`, `sudoku`, `life`, `catclock`, `juggle` and `festoon`. Where a program seeds from the time or `/dev/random`, its script is masked. If masking is not enough, the script is smoke. A program with a seed flag runs with a fixed seed.
- **Animation timing.** Games draw frames on a timer. Scripts snap only after `quiet`, or after a fixed number of frames where the program counts them (`doom -timedemo`).

**`doom`.** `doom -timedemo demo1` with the freedoom WAD (BSD-licensed) runs the same game tics regardless of speed. After the demo, it prints the tic count, and the final frame is deterministic. The script snaps the final frame (exact) and records the reported frames per second, which goes to 13, not here.

**Emulators.** The emulators are `games/nes`, `snes`, `gb`, `gba`, `c64`, `md`, `2600` and `v8e`. Each runs a free homebrew ROM for a fixed number of frames with scripted input, using its own frame-count option if it has one. A ROM is chosen per emulator from published homebrew with a licence that allows redistribution. The list goes in `gui/roms/LICENSES`. Without a frame count, the script is masked or smoke.

**Audio.** Programs that play sound are `zuke`, `games/doom`, `games/midi`, `games/opl3` and the emulators. They are checked by capturing `/dev/audio` writes. In the guest, a helper binds a file server over `/dev/audio` in the program's namespace, so the program writes to a file. R's bytes must equal G's for the deterministic decoders. For the rest, they are smoke.

## 12.7 The 2x2

When R fails a boot, smoke, `mk test`, `diffrun` or GUI test, the failure is attributed before it is filed. The same test runs on all four combinations:

| | G user space | R user space |
|---|---|---|
| **G kernel** | the oracle | **user**: R's user code, G's kernel |
| **R kernel** | **kernel**: R's kernel, G's user code | R |

- If only "user" fails, the bug is in R's user-space code, the libraries, or the program.
- If only "kernel" fails, the bug is in R's kernel.
- If both fail, R's `libc` or another library both sides use is the first suspect.

For GUI tests on H, the kernel is the `9h` kernel built by R or G (10.2). `r9 2x2 <test>` builds the four images from the existing build outputs, with no new compile, and runs them.

## 12.8 Mixed-object bisection

Once the 2x2 has found the side, `r9 mix` finds the unit. All objects link together (05.7), so `r9 mix bisect <test> --side=user --prog=acme` does the following:
1. Start with all of `acme`'s objects from G and its libraries from G. The test must pass.
2. Swap halves of the units to R's objects and relink with G's `6l`.
3. Rerun the test, and recurse on the half that brings back the failure.

Libraries are bisected at the object level inside the archive. The kernel is bisected the same way, over port and arch objects, and every step boots.

Bisection ends at one unit. `r9 sdiff <unit>` then shows R's and G's code side by side, function by function, from the objects. Most miscompiles are found in one function within an hour. The unit, the function and a reduced test go into a `rucc-compat` case (15.2) before the fix.

When a test fails only with several R units together, which is an ABI or layout disagreement, bisection says so. It reports the minimal set it found, and `r9 objcheck --pair` compares the two units' views of every shared structure through their acid output (07.7).

## 12.9 Checks

| Claim | Check |
|---|---|
| G is stable on every graded test | `r9 baseline --runs=3` (12.1) |
| R passes every graded `mk test` case G passes | `r9 test --suite=mk` |
| R kernels boot and pass the smoke suite | `r9 boot --objtype=amd64,arm64` |
| kencc built by rucc reproduces G byte for byte | `r9 fixedpoint` |
| The 40 `diffrun` programs agree | `r9 diffrun` |
| The GUI suite meets 02.7's counts | `r9 gui --kernel=R --user=R --host=q-amd64,h-m4,h-gpc` |
| Every failure filed has a 2x2 and a bisected unit | `r9 report --check-filing` |
