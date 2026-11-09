# Where rucc stands

This is the gap inventory on 9 November 2026, against rucc 0.30.15 at `tamnd/rucc` main `5f9810aab`. Every claim here was checked in one of four ways:
- with a release build of that commit (`/tmp/rucc-main/target/release/rucc`, on the M4 builder);
- against GCC 16.2.0 (`/opt/homebrew/bin/gcc-16`);
- against the 9front tree at the pin, cloned on 9 November 2026;
- by reading the named file in rucc or in 9front.

Nothing has been compiled for Plan 9 yet, because rucc cannot target it (03.1). The probes are host-target `-fsyntax-only` runs and small files. They show what the front end accepts, not what it generates.

Sizes follow bsd 03:

| Size | Engineer time, including corpus cases and reductions |
|---|---|
| **S** | under 3 days |
| **M** | 1 to 2 weeks |
| **L** | 3 to 6 weeks |
| **XL** | more than 6 weeks, or a new subsystem |

## 03.0 The headline

rucc already has these parts:
- a C front end that parses most of 9front's C;
- an optimizer and two back ends for the right instruction sets;
- the kernel features `rucc-kernel` built;
- a `DataModel::Llp64` with 32-bit `long` and 64-bit pointers. Today only Windows uses it, and it is exactly Plan 9's model on amd64 and arm64.

What it does not have is everything that makes Plan 9 a different platform:
- the target;
- the dialect;
- the calling convention;
- the object format;
- the way the build reaches the compiler.

Four of these are new subsystems, not fixes: the kencc ABI, the `.6` writer, the `.7` writer, and the 9P sysroot. That is why the visible total is larger than the BSD folder's.

| Area | State | Size | Milestone |
|---|---|---|---|
| 03.1 Plan 9 targets | refused | M | P1 |
| 03.2 The dialect | partly accepted, one silent wrong-code mode | L | P1, P2 |
| 03.3 Brace elision over pointer arrays | a real C bug, also hits GCC targets | S | P1 |
| 03.4 The kencc calling convention | absent | L | P2 |
| 03.5 `.6` and `.7` object writers | absent | XL | P2, P4 |
| 03.6 Plan 9 assembler syntax (`6a`, `7a`) | absent | L | P6 |
| 03.7 9P sysroot | absent | M | P1 |
| 03.8 `extern register` | refused (E0714) | M | P2 |
| 03.9 kencc's arithmetic conversions | ANSI rules, which give different results | M | P2 |
| 03.10 Preprocessor differences | ANSI behaviour | S | P1 |
| 03.11 `-fplan9-extensions` (GCC flag) | unknown option | S | P1 |
| 03.12 Format checking from `varargck` | pragmas ignored | M | P3 |
| 03.13 Signatures (`-T`, `signof`) | absent | M | P3 |
| 03.14 Lax types: enums, qualifiers, implicit int | errors where kencc accepts | S | P2 |
| 03.15 Lexer: UTF-8 identifiers, `0x…+1` | partly | S | P1 |
| 03.16 `vlong` and runtime helpers | absent for this ABI | S | P2 |
| 03.17 `9h` | nothing beyond 03.1 to 03.16 and the kernels' needs (09.3) | none | none |

## 03.1 Plan 9 targets are refused

```
$ rucc --target=x86_64-plan9 -c t.c
rucc: error: unsupported target triple `x86_64-plan9`: unknown operating system
$ rucc --target=amd64-unknown-plan9 -c t.c
rucc: error: unsupported target triple `amd64-unknown-plan9`: unknown operating system
```

There are two `Os` enums: `crates/rucc-tuple/src/os.rs:15` and `crates/rucc-target/src/lib.rs:161`. Neither has Plan 9, and `docs/TARGETS.md` has no Plan 9 row. The work:
- `Os::Plan9` in both enums.
- Two triples, `x86_64-unknown-plan9` and `aarch64-unknown-plan9`. These accept the aliases `amd64-plan9` and `arm64-plan9`, and the `objtype` names alone (`--target=amd64`) when invoked as `6c` or `7c` (05.3). Later, `i386-unknown-plan9` and `arm-unknown-plan9`.
- Data model `Llp64` on both 64-bit targets. The doc comment in `crates/rucc-tuple/src/arch.rs:236` ("Windows, and only Windows") changes.
- `char` is signed on every Plan 9 target, arm64 included. That differs from AAPCS64, and kencc's `tab` in `cc/sub.c` and APE's `limits.h` (`CHAR_MIN = SCHAR_MIN`) both confirm it.
- No predefined macros. kencc predefines nothing: the `objtype`-specific headers come from `/$objtype/include`, and APE's `cc` adds `-D_POSIX_SOURCE` and similar on its own command line. rucc's Plan 9 targets predefine nothing either, apart from `__RUCC__`-style macros under `-fno-kencc-pure`, which is off by default (05.3).
- The `Convention` of 03.4 and the object format of 03.5.
- `docs/TARGETS.md`: four new rows at tier "corpus", with the reference "kencc at the 9front pin".

Size M, in P1.

## 03.2 The dialect

Each item was probed with a minimal file. The tree counts are from the pin.

**Unnamed struct members.** In the tree, 1,177 lines in 388 files declare an unnamed member by type name: `QLock;` 34 times, `Ref;` 31, `Lock;` 27, and so on. Probe (`anon.c`):

```c
struct Lock { int key; };
struct Ref { Lock; long ref; };
long incref(Ref *r) { lock(r); r->key = 1; r->Lock.key = 2; return ++r->ref; }
```

| Mode | Result |
|---|---|
| plain | `warning: declaration does not declare anything [E0547]`, then `no member named 'key'` and `no member named 'Lock'` |
| `-fms-extensions` | `r->key` works. `lock(r)` is an error: `passing argument 1 of 'lock' from incompatible pointer type [E0512]`. `r->Lock` is still "no member" |
| `-fms-extensions -std=gnu89` | `lock(r)` becomes a warning, and the pointer is passed **unadjusted**. With `Lock` first in `Ref` the offset happens to be 0. With it anywhere else, this is silent wrong code |
| `-fplan9-extensions` | `unknown option` |
| GCC 16 `-fplan9-extensions` | accepted: promotion with the offset applied, and `r->Lock` works |

kencc goes further than GCC in three ways (06.2):
- the member name is the type name after `typedef`;
- promotion applies on assignment and on argument passing, to any depth;
- a name conflict between levels is resolved silently, and the manual calls the result "unspecified".

`cc/dcl.c` decides it: the first match in a breadth-first search wins. Size M for the GCC flag. The kencc extras and the `gnu89` hazard come to another M.

**`USED`, `SET`.** These appear 971 and 138 times. rucc says `implicit declaration of function 'USED' [E0521]`. kencc treats them as keywords taking any number of arguments, used only to silence warnings (`cc/lex.c:1150-1188`). They must be keywords in `-fkencc` mode, not macros, because the tree defines no macro for them and `#ifdef USED` must stay false. Size S.

**`signof`, `typestr`.** These appear in 5 files. rucc gives a parse error at `signof(struct S)`. `signof` is the 32-bit signature hash of a type (07.4), and `typestr` is the type's printed form as a string constant. Size S once the signatures of 03.13 exist.

**Compound values and designators.** kencc accepts:
- `(struct S){…}` as a value anywhere;
- the old designators `[3] 1` and `.y 1`, with no `=`.

rucc accepts the C99 forms. The old forms appear in `9/port` and in the `pc` drivers. Size S.

**`#pragma`.** The pragmas and their counts are `varargck` 554, `incomplete` 110, `lib` 100, `src` 54, `pack` 50, `profile` 28, `interface` 26, `once` 24, and `warning` 10. `ref`, `implementation`, `unused`, `export`, `optimize`, `message` and `argsused` are rarer. rucc ignores all of them silently except `pack`, where it says `missing '(' after '#pragma pack' - ignored [E0677]`.

Three of them change output:
- `pack on|off` changes struct layout;
- `lib` names a library, which `6l` adds to the link from the object file;
- `profile off` disables profiling code.

`varargck` drives format checking (03.12), and `incomplete` affects only `acid` output and warnings. kencc acts on just six pragmas: `lib`, `pack`, `fpround`, `profile`, `varargck` and `incomplete` (`cc/macbody`, `macprag`). It ignores every other one, `once` included, so rucc must not honour `#pragma once` in `-fkencc` mode (06.5). Size M.

**The silent wrong-code mode is the most important finding in this document.** A build that "just" adds `-fms-extensions -std=gnu89` compiles a large part of the tree and passes members at the wrong address. That is exactly the bring-up shortcut a hurried implementer would take. rucc must reject promotion in every mode except `-fplan9-extensions` and `-fkencc`, and give an error, not a warning, for the `gnu89` case on Plan 9 targets. P1 lands the error before anything else.

## 03.3 Brace elision over pointer arrays: a real C bug

```c
char *d[2][2] = { "a", "b", "c", "d" };     /* from cmd/xd.c:41 */
```

rucc:
```
error: cannot initialize array of 'char *' from a string literal with type array of 'char' [E0638]
warning: excess elements in array initializer [E0635]
```

GCC 16 accepts it as `{{"a","b"},{"c","d"}}`, as C requires (C11 6.7.9p20). The bug is in rucc's brace-elision walk: it treats a string literal as the initializer of a char array before it checks the element type. It affects every target, so it is a GCC-compatibility bug in its own right. It gets a `rucc-compat` case. Size S, in P1.

## 03.4 The kencc calling convention is absent

`crates/rucc-abi/src/abis.rs:708`'s `Convention` has `Target`, `Ms`, `Sysv`, `Stdcall`, `Fastcall` and `regparm`. Plan 9's convention is none of them (08.2):
- **Arguments.** All arguments are in stack slots in the caller's frame. The first is also passed in `RARG` (`BP` on amd64, `R0` on arm64). The callee stores it to its slot if its address is taken, and that is what `va_start` relies on.
- **Return values.** They come back in `AX`/`R0`, and floats in `X0`/`F0`. Structs come back through a hidden pointer passed as the first argument.
- **Saved registers.** None. Every register is caller-saved, except the `extern register` ones, which nobody touches. `jmp_buf` is two words: PC and SP.
- **Frames.** No frame pointer. The return address sits directly below argument 0, and `getcallerpc(&firstarg)` reads `((uintptr*)&firstarg)[-1]`.
- **Small arguments.** Arguments narrower than 8 bytes take a full 8-byte slot on amd64 and arm64, and `va_arg` in `u.h` indexes into the slot as a little-endian value.

Struct layout differs as well, and that matters more than the calls, because it reaches every program. It is decided by `align()` (`6c/swt.c` and `7c/swt.c`) and `sualign()` (`cc/dcl.c`):
- every struct and union is padded to a multiple of 8 bytes (`Asu2`), so `sizeof(struct{char c;})` is 8;
- a member of struct, union or 8-byte type is aligned to 8 (`Ael1`, from `ewidth[TSTRUCT] = -1`);
- bit fields are allocated in `int` units on amd64 and `long` units on arm64 (`tfield`, `6c/txt.c:23`, `7c/txt.c:24`);
- `#pragma pack` replaces all three widths with its value.

None of rucc's layouts does this, so it needs a `Layout::Kencc` in `rucc-types` (08.1).

This is a new `Convention::Kencc` with a per-architecture description, together with the layout. Size L, because every lowering of calls, varargs, struct returns and frames needs the third variant. The parts already written for `Ms` and `Sysv` are the template.

## 03.5 The `.6` and `.7` object writers are absent

`crates/rucc-object/src` has `elf.rs`, `macho.rs`, `coff.rs` and `wasm.rs`. Plan 9 objects are none of these (07.1). They are a stream of `Prog` records:
- an opcode number from `6.out.h` or `7.out.h`;
- addressing-mode records with symbol references by small index;
- `ANAME` symbol declarations;
- `AHISTORY` file and line history;
- `ATEXT` and `AGLOBL` with frame size and flags.

`6l` and `7l` do the instruction encoding, the branch layout, the prologues and the stack-split checks. So rucc does not emit bytes here. It emits the linker's input language, and its own instruction selection must produce only the instructions and modes the linker's `optab` encodes.

This is the single largest piece of rucc work. Its size is XL:
- an object writer for each architecture;
- a `Prog`-level instruction set in the back end, which is a new "MC layer" alongside the machine-code emitters;
- a check that every `Prog` emitted has an `optab` entry (07.3).

## 03.6 Plan 9 assembler syntax is absent

`crates/rucc-asm/src` reads AT&T (`att.rs`), Intel (`intel.rs`), A64 (`a64.rs`), RISC-V (`rv.rs`) and 16-bit x86 (`sixteen.rs`). It does not read Plan 9 assembler syntax:
- `MOVQ $1, AX`;
- `TEXT name(SB), $framesize`;
- `DATA`/`GLOBL`;
- pseudo-registers `SB`, `FP`, `SP` and `PC`.

There are 35,898 lines of `.s` in the tree. They are built by the tree's own `6a`/`7a` until P6, and counted as `ref-as` (02.2). A Plan 9 assembler front end that writes the same `.6` records through 03.5's writer is size L, in P6.

## 03.7 There is no 9P sysroot

`crates/rucc-sysroot/src` resolves headers on the host file system. The build runs in a guest, and rucc runs on the host (05). rucc needs a sysroot provider that reads a 9P2000 file tree over TCP:
- `--sysroot=9p:tcp!host!port`;
- with a session that keeps `fid`s open across compiles;
- with a cache keyed by `qid.path` and `qid.vers`, so a header read by one compile is not read again by the next.

This gives rucc `rucc-9p`, a 9P2000 client with no other dependencies. It is about 1,200 lines. Size M, in P1. Before it lands, P0 and the first part of P1 use the kernel 9P mount on Linux builders (`mount -t 9p -o trans=tcp,version=9p2000`), which needs nothing from rucc.

## 03.8 `extern register` is refused

```c
register struct Mach *m asm("r15");
```
gives `error: 'm' is a global register variable, which this compiler does not support [E0714]` (`crates/rucc-sema/src/check/decl.rs:2132`).

The kernel declares `extern register Mach *m; extern register Proc *up;` in each `dat.h`. 54 of the 962 sampled files fail on it, which is all of `9/port` and the kernel parts of `9/pc`. kencc allocates these registers from `REGEXT` downward:
- amd64: `R15` for the first and `R14` for the second;
- arm64: `R27` and `R26`.

They are reserved in every function the kernel compiles, not only those that use them. The work:
- parse kencc's form, with no `asm("reg")`, in `-fkencc` mode;
- parse GCC's form everywhere;
- reserve the registers in the allocator for the whole translation unit;
- make loads and stores of the variable register moves.

Size M, in P2. The GCC form is a gap against GCC 16 too, so this item also closes a GCC-compatibility hole.

## 03.9 kencc's arithmetic conversions give different answers

`2c(1)` says the compilers use "the original unsigned preserving" rules. `cc/sub.c`'s `tab[NTYPE][NTYPE]` (around line 1852) makes this exact, and it reaches further than promotion:
- `uchar` with `int` gives `uint`;
- `ushort` with `int` gives `uint`;
- `char` with `uchar` gives `uchar`, and then `uint` after the "at least int" step in `arith()`.

So `(c - 1) < 0` with `uchar c = 0` is false in kencc and true in ANSI C. `c < i` with `int i = -1` compares unsigned. Code in the tree may depend on this, deliberately or by accident. A rucc that follows ANSI silently changes behaviour.

The work:
- the conversion table as a property of `-fkencc`;
- a diagnostic, `-Wkencc-conversion`, that rucc can run on any C to count expressions whose type differs between the two rules in an observable way: comparison, division, right shift, conversion to a wider type. P1's census runs it over the tree, and that tells us how much code is at stake.

Size M, in P2.

## 03.10 Preprocessor differences

kencc's built-in preprocessor handles `#ifdef`, `#ifndef`, `#else`, `#endif`, `#line`, `#define`, `#include`, `#undef` and `#pragma`. It has no `#if` unless `-p` is given, in which case it runs the full `cpp` (`/sys/src/cmd/cpp`). 495 files contain `#if`. The mkfiles that pass `-p` cover all of them:
- `proof`, `reform`, `gs`, `bzfs`, `audio`, `bzip2`, `git`, `ext4srv`;
- `boot/efi`, `boot/pc` and `9/xen`.

Without `-p`, `#if` is an error in kencc. rucc must give the same error, because a file that only builds under rucc is not "configured the same".

Two kencc behaviours differ from rucc's:
- **Macro arguments inside strings.** `#define STR(x) "x is x"` with `STR(hello)` gives `"hello is hello"` in kencc and `"x is x"` in rucc (`strsub.c`). This is off under `-p`, because `cpp` is ANSI.
- **Include lookup.** The order is the file's own directory, then each `-I`, then `/sys/include`, then `/$objtype/include`. `-.` removes the file's own directory.

Both are part of `-fkencc` in `rucc-pp`. Size S.

## 03.11 `-fplan9-extensions` is unknown

``rucc: error: unknown option `-fplan9-extensions` ``. GCC 16 has had it since GCC 4.6. Its semantics:
- unnamed members of a tagged struct or typedef'd type;
- implicit conversion of a pointer to the containing struct into a pointer to the unnamed member, on assignment and argument passing;
- access by the typedef name.

It is a GCC flag, so rucc owes it under the parent goal regardless of Plan 9. Size S, since the work is shared with 03.2.

## 03.12 Format checking from `varargck`

There are 554 `#pragma varargck` lines. They declare:
- `print`-like functions, with `argpos`;
- the type of each verb, with `type "s" char*`;
- flag characters, with `flag`.

`-F` turns checking on, and every build passes `-F` through `CFLAGS=-FTVw`. rucc ignores the pragma (`prag.c`: `print("%s", 1)` gives no diagnostic). The tree registers its own verbs too: `%r`, `%C`, `%S`, `%H`, `%I`, `%E` and `%V`. They come from `fmtinstall` and are declared with `varargck type`.

This is not just about warnings. `-w` turns the warnings on, and the build does not fail on them. They are part of "configured the same", though: `r9 warn-diff` compares R's diagnostics with G's per unit, as a report. It is never a gate, because kencc's warnings are not errors (05.8). Size M, in P3.

## 03.13 Signatures: `-T` and `signof`

With `-T`, kencc emits a type signature for every external function and data symbol. `6l` checks that the signatures agree across objects, and a mismatch is a link error: "incompatible type signatures". The signature is a 32-bit hash over the type's structure (`cc/sub.c`, `signature()`).

rucc must produce the same hash bit for bit, or `6l` will reject the mix of R and G objects in the mixed tests (12.8). It also needs the hash for `signof`. Size M, in P3. Its test is a table of 2,000 types from the tree hashed by both compilers, in `rucc-compat`.

## 03.14 Lax types

kencc accepts things that rucc, like GCC, rejects:
- **Enums are compatible with `int`.** `void f(enum E); void f(int x){}` gives `conflicting types for 'f'` in rucc (`en.c`). The tree has it in `draw.h`'s `Drawop` prototypes against `int` definitions, 4 cases in the sample.
- **Qualifiers are lax.** APE's `stdio.h` has `perror(const char*)` and `libc.h` has `perror(char*)`.
- **Implicit `int`.** `static pcmp(...)` in `mk`, and `mainstacksize = 64*1024;` at file scope in `flambe.c`.
- **`true` is an ordinary identifier** (`6c/cgen.c`, `dd.c`). C23 made it a keyword, so `-fkencc` implies a pre-C23 keyword set.
- **Duplicate members.** In the sample, 44 files fail in rucc and 138 in GCC on `rio`'s `dat.h:144` and similar headers. An unnamed member's fields there collide with a named field. kencc allows this and resolves by depth (06.2).

Each is a mode switch in `rucc-sema`. Together they are size S.

## 03.15 The lexer

- **UTF-8 identifiers.** `µs(` appears 40 times, along with `søkrisee` and `fmtæ`. rucc accepts them. GCC 16 gives `stray '\302' in program` in 11 files.
- **`0xCAB00D1E+1`** (`libc/port/pool.c:99`). kencc lexes this as hex followed by `+1`. rucc and GCC both say "invalid suffix", because C's pp-number grammar swallows `E+1`. In `-fkencc` mode, the lexer ends a hex number at `E+`. Size S.

## 03.16 `vlong` and the runtime helpers

kencc calls these helpers for some 64-bit operations on 32-bit targets, and for a few on 64-bit ones:
- `_divv`, `_modv`, `_divvu`, `_modvu`;
- `_v2f` and the other conversions.

They are defined in `libc/$objtype/vlop.s` and `vlrt.c`. On amd64 and arm64, 6c and 7c inline almost all of them. rucc emits only what the target's `libc` provides, and checks it against the symbol list `6l` would resolve (08.6). Size S. It is larger on 386 and arm, later.

## 03.17 The host build of `9h`

The `9h` kernel is a Plan 9 kernel. It is built in the guest by `6c` or `7c`, so R builds it with the same rucc as `pc64` and the arm64 `qemu` kernel, with the same needs (09.3). The `9h` monitor is Rust. So `9h` adds no rucc work.

An earlier draft compiled port for the host, with a new `-mlong32` flag that gave an LLP64 data model on an LP64 host target. 10.1 explains why that design was dropped. The flag is not needed.

## 03.18 The mass probe

All 962 files of the sample were compiled `-fsyntax-only` against the tree's headers:
- **The sample:** libc, libdraw, libmemdraw, libthread, libbio, lib9p, libsec, libmp, libframe, libplumb, libregexp, libauth, acme, rio, sam, samterm, rc, mk, 9/port, cc, 6c, 6l and `cmd/*.c`.
- **Include paths:** `-I/sys/include -I/amd64/include`, both mapped to the clone.
- **Scripts:** `/tmp/p9scripts/one.sh` and `oneg.sh`.

| Compiler and mode | Pass | Fail | Top first errors |
|---|---|---|---|
| rucc, plain | 421 | 541 | file not found 149; "flexible array member in a struct with no named members" 145; implicit `USED`/`SET` 112; "multiple storage classes" (`extern register`) 54; no member 45 |
| rucc `-fms-extensions`, `USED`/`SET` as macros | 569 | 393 | |
| same, `-std=gnu89` | 687 | 275 | not found 149; multiple storage classes 54; duplicate member 44; no member 14; conflicting types 4; `Rune` array init 3; pp-number 1 |
| GCC 16 `-std=gnu89 -fplan9-extensions`, `USED`/`SET` as macros | 685 | 277 | duplicate member 138; `os.h` missing 78; `stdlib.h` missing 17; stray `\302` 11 |

The "not found" failures are not compiler failures. They come from the per-directory `-I` flags in each mkfile (`os.h`, `mesg.h`, `io.h`, `screen.h`) and from APE's include path. In the real build, the mkfile supplies them.

Leaving those aside, the remaining failures are all in 03.2, 03.8 and 03.14, so the front end is close. The back end, the ABI and the object format are where the work is.

## 03.19 Sizes in total

| Milestone | rucc work from this document |
|---|---|
| P1 | targets (M), the dialect's front half (M), brace elision (S), 9P sysroot (M), preprocessor (S), `-fplan9-extensions` (S), lexer (S) |
| P2 | kencc convention (L), `.6` writer and the `Prog` layer for amd64 (XL, first half), `extern register` (M), conversions (M), lax types (S), helpers (S) |
| P3 | `varargck` (M), signatures (M) |
| P4 | `.7` writer (XL, second half) |
| P6 | Plan 9 assembler syntax (L) |

The visible total is about 28 to 38 engineer-weeks of rucc work. Unseen code generation bugs come on top, along with whatever the census (05.6) and the conversion count (03.9) find. Document 16 adds the harness and `9h` work and a contingency.
