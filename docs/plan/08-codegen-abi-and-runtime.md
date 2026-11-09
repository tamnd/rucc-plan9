# Code generation, the ABI and the runtime

This document defines the Plan 9 targets for rucc's back ends:
- the data model;
- struct layout;
- the calling convention;
- the reserved registers;
- system calls;
- process start-up;
- the runtime helpers;
- what the optimizer may assume.

Every rule cites the code in the tree that decides it. Where rucc must reproduce a rule bit for bit, the document says so. The test is mixed linking with G objects (12.8): a layout or convention difference shows up there as a wrong result in a program that mixes R and G objects.

## 08.1 Data model and layout

From `6c/gc.h` and `7c/gc.h`, which agree:

| Type | Size | Alignment as a member | Alignment as a parameter slot |
|---|---|---|---|
| `char`, `uchar` | 1 | 1 | 8 |
| `short`, `ushort` | 2 | 2 | 8 |
| `int`, `uint`, `long`, `ulong` | 4 | 4 | 8 |
| `vlong`, `uvlong` | 8 | 8 | 8 |
| `float` | 4 | 4 | 8 |
| `double` | 8 | 8 | 8 |
| pointer | 8 | 8 | 8 |
| `struct`, `union` | rounded up to 8 | 8 | rounded up to 8 |
| `Rune` | 4 (`uint`) | 4 | 8 |

`char` is signed. `long` is 32 bits, so this is LLP64. rucc's `DataModel::Llp64` gives the sizes, but not the struct rules:
- **Every struct and union size rounds up to 8** (`Asu2`), whatever its members. `sizeof(struct{char c;}) == 8`.
- **A member of struct or union type aligns to 8** (`Ael1`, because `ewidth[TSTRUCT] = -1` falls into "`w <= 0` gives `SZ_VLONG`"). An array member aligns as its element.
- **Bit fields** are allocated in units of `tfield`: `int` on 6c and `long` on 7c, which are both 4 bytes. They go from the low bit upward, and a field that does not fit starts a new unit. `cc/dcl.c`'s `sualign` and the `shift` sign trick in `dcl.c` decide the exact placement, and rucc reproduces them.
- **`#pragma pack on`** sets all three widths to 1. `pack N` sets them to N.
- **Unnamed members** are laid out as ordinary members (06.2).

So rucc's Plan 9 targets need a new `Layout::Kencc` in `rucc-types`. It is tested by the `acid` files of 07.7, which print every offset, and by `rucc-compat/plan9/layout/`: 3,000 struct types from the tree, with `sizeof`, `offsetof` and bit-field positions, compiled by G and by rucc.

## 08.2 The calling convention

From `6c/gc.h` and `6.out.h` (amd64), and `7c/gc.h` and `7.out.h` (arm64):

| | amd64 | arm64 |
|---|---|---|
| First argument also in | `RARG` = `BP` | `R0` |
| Integer and pointer return | `AX` | `R0` |
| Float return | `X0` | `F0` |
| Stack pointer | `SP` | `R31` |
| Static base | none | `R28` (`REGSB`), set to `setSB` by `_main` and never changed |
| Linker temporary | `DI` (`REGTMP`) | `R17` (`REGTMP`); `R16` is also used by `7l` |
| Link register | none (`CALL` pushes) | `R30` (`REGLINK`) |
| Allocatable by the compiler | the rest | `R9`–`R23` (`REGMIN`–`REGMAX`); `F7` and up (`FREGMIN`) |
| External registers (`extern register`) | `R15`, `R14`, … down | `R27`, `R26`, … down; floats from `F15` down (`FREGEXT`) |
| Callee-saved | **none** | **none** |

**Arguments.** The caller reserves a slot for every argument at the bottom of its outgoing area, including the first. Each slot is 8 bytes, or a struct's size rounded up to 8 (`Aarg1`/`Aarg2`). Small values sit at the start of the slot (little-endian), and `va_arg` in `u.h` reads them with that layout. The caller passes the first argument in `RARG` as well as, for a non-struct, in the slot. The callee stores `RARG` back to the slot only if it needs to. Variadic functions always do, because `va_start(list, start)` takes `&start`. On amd64, argument 0 is at `0(FP)`, which is `8(SP)` at entry, directly above the return address. On arm64 there is no pushed return address. `7l`'s prologue stores `R30`, and argument 0 is at `8(FP)`. rucc takes the exact offsets from `7l/noop.c` and does not invent them.

**Struct arguments** are copied into their slots. **Struct returns** use a hidden pointer passed as the first argument (`Aarg0` reserves the slot). The callee returns the pointer in `AX`/`R0`.

**No callee-saved registers.** Every register except the external ones, `SP`, `R28` on arm64, and the linker temporaries may be clobbered by any call. `setjmp` stores two words, SP and the return PC (`libc/amd64/setjmp.s`, `libc/arm64/setjmp.s`), and `jmp_buf` is `uintptr[2]`. rucc's register allocator treats every register as caller-saved on these targets. That is the main runtime cost of this ABI, and it is part of why kencc's code is simple. rucc can still win by keeping values in registers between calls and spilling around them.

**`getcallerpc`** is `MOVQ -8(RARG), AX` on amd64: it reads the word below argument 0, which is the return address. rucc must keep that layout in every non-leaf and leaf function, which follows from using the same frame scheme. The kernel's lock debugging and `qlock` owner tracking depend on it.

**Frames** have no frame pointer. The `TEXT` frame size is the locals plus the outgoing argument area, and `6l` and `7l` insert the `SP` adjustment (07.2). Unwinding for `acid`, `snap` and the kernel's `dumpstack` uses `6l`'s and `7l`'s stack-size table (`pc/sp`), which the linker computes from the frame sizes and the `SP` adjustments it sees. So rucc must never move `SP` inside a function body except by `Prog`s the linker understands:
- `PUSHQ` and `POPQ`, which 6l tracks;
- `SUBQ` and `ADDQ` of a constant on `SP`.

`alloca` does not exist in kencc, and rucc rejects it on Plan 9 targets. Dynamic stack adjustment is not used.

**A new `Convention::Kencc`** in `crates/rucc-abi/src/abis.rs` holds these rules as data per architecture. Its tests are:
- `rucc-compat/plan9/abi/`, with 400 call shapes from GCC's `struct-layout-1` and `compat` suites translated to Plan 9 types, built with one side R and the other side G, in both directions;
- the mixed-object runs of 12.8.

## 08.3 Floating point

There are no float arguments in registers. Floats go in their 8-byte slots, as `double` when unprototyped. `float` parameters with a prototype stay `float` (4 bytes in an 8-byte slot). Return values are in `X0` and `F0`. The FPU control word and the exception mask are set by `libc`'s `main9` and `setfcr` (`libc/amd64/getfcr.s`). rucc must not assume a default rounding mode beyond what `FLT_ROUNDS` reports at run time, and it may not reorder across calls to `setfcr`, `getfcr` or `setfsr`. These are ordinary calls to rucc, so they already act as barriers, provided no float operation is hoisted across a call. The rule for `-fkencc`: floating-point operations are not moved across calls.

## 08.4 Reserved registers

The external registers of 06.9 are reserved in every function on Plan 9 targets, as `-ffixed-r15 -ffixed-r14` (amd64) or `-ffixed-x27 -ffixed-x26 -ffixed-x28` (arm64; `x28` is `SB`). This applies to user programs as well as kernels, because `6l` and `7l` reserve them for whole programs. The kernel declares only two external registers (`m` and `up`), but the convention reserves the range down to the lowest one a unit declares. rucc reserves exactly the registers kencc would have assigned, by counting `extern register` declarations in the unit. It also always reserves two on the kernel targets, because the kernel's assembly (`l.s`) loads them by number.

## 08.5 System calls and start-up

**System calls** are generated stubs (`libc/9syscall/mkfile`). On amd64:

```
TEXT open(SB), 1, $0
	MOVQ RARG, a0+0(FP)
	MOVQ $14, RARG
	SYSCALL
	RET
```

The arguments are on the user stack at `8(SP)`, the number is in `RARG`, and the result is in `AX`. On arm64 it is `MOV R0, 0(FP); MOV $n, R0; SVC; RETURN`. These are `.s` files built by the tree's assembler (07.5), so rucc's only obligation is to call them with the convention of 08.2. That is what `9h` implements on the other side (10.5). There are 52 entries in `sys.h`.

**Start-up.** On amd64, `_main` (`libc/amd64/main9.s`) receives the `Tos` pointer in `AX` from the kernel and stores it in `_tos`. It then calls `_callmain(main)`, which sets up `argv0`, the `atexit` list and the profiling hooks, and calls `main(argc, argv)`. `Tos` (`/sys/include/tos.h`) sits at the top of the user stack. It holds the profiling links, `cyclefreq`, `kcycles`, `pcycles`, `pid` and `clock`, and `9h` must fill it exactly as the kernel does (10.5). On arm64, `_main` also sets `R28` to `setSB`.

**Executables** are Plan 9 `a.out` (`/sys/include/a.out.h`): a 32-byte header of big-endian words, `magic`, `text`, `data`, `bss`, `syms`, `entry`, `spsz` and `pcsz`, plus a 64-bit entry for `HDR_MAGIC` formats. Magic is `S_MAGIC` for amd64 and `R_MAGIC` for arm64. `UTZERO` is `0x200000` on pc64 (`9/pc64/mem.h`), and the text starts at `UTZERO + sizeof(Exec)` plus the extra 8 bytes for `HDR_MAGIC`. The `6l` and `7l` that rucc does not replace write all of this. rucc's only part is the input.

## 08.6 Runtime helpers

On amd64 and arm64, `6c` and `7c` generate inline code for all `vlong` arithmetic. Struct copies are inline too: `6c/cgen.c:1772` emits `REP MOVS`. The compiler-generated calls that remain are:
- the profiling hooks `_profin` and `_profout`;
- on 386 and arm, the `vlong` helpers in `libc/$objtype/vlrt.c` and `vlop.s`.

The exact list per target is measured, not assumed.

`r9 helpers` lists every external symbol G's objects reference that does not appear in any source as a C identifier. That list is the compiler-generated calls. rucc may call only symbols in that list, and must call each with the same convention. rucc's own struct-copy and memset lowering emits inline code or calls `memmove` and `memset`, never `memcpy` with an aliasing assumption. Plan 9's `memcpy` is `memmove` in `libc/amd64/memcpy.s`, but rucc does not rely on it.

## 08.7 What the optimizer may assume

kencc never exploited undefined behaviour, and 9front's code is written for a compiler that does not. On Plan 9 targets, `-fkencc` makes these true:
- `-fwrapv`: signed overflow wraps;
- `-fno-strict-aliasing`;
- `-fno-delete-null-pointer-checks`: page 0 is unmapped in user space, but the kernel reads low addresses deliberately;
- no reasoning from out-of-range shifts or from reading uninitialised variables;
- `-fno-builtin` for everything except `memmove`, `memset`, `memcmp`, `strlen` and `strcmp`, which rucc may inline because their Plan 9 semantics are the ANSI ones;
- no library calls invented: no `memcpy`, no `__stack_chk`, no `__udivti3`;
- volatile is respected as kencc does it: every access is a memory access, in program order.

With these, rucc at `-O2` still has the things that make it faster than kencc:
- inlining within a unit;
- global register allocation across basic blocks;
- loop optimizations;
- instruction scheduling;
- better instruction selection.

Open question 3 asks whether the tree is clean enough for strict aliasing to be turned on later for user space as a performance option. It is never on in graded runs.

## 08.8 The `9h` kernel

The `9h` kernel is built like `pc64` and the arm64 `qemu` kernel, by `6c`/`7c` in the guest, with the same convention, layout and reserved registers. Nothing in this document changes for it.

An earlier draft compiled port for the host with a `-mlong32` flag that gave an LLP64 data model on an LP64 host target. That design was dropped (10.1), and rucc gains no such flag.
