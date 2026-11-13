# The assembler and the object format

Plan 9 splits the work of a toolchain differently from Unix. The compiler and the assembler emit the same thing: a stream of abstract instructions (`Prog` records) in a `.6` or `.7` file. The linker, `6l` or `7l`, chooses the encodings, lays out the code, resolves branches, inserts the stack adjustment that the `TEXT` frame size asks for, and writes the executable. rucc has to emit that stream, because the tree's linker reads it and nothing else.

## 07.1 The `.6` file

`6c` writes the file in `outcode` (`cmd/6c/swt.c:178`). The format is a header-less sequence of records.

**The record header** is 6 bytes: the opcode `as` as 2 little-endian bytes, then `lineno` as 4 bytes. The opcodes are from the `enum as` of `6c/6.out.h`, which has 680 entries up to `ALAST`.

**Then two address records,** `from` and `to`, each written by `zaddr` (`swt.c`). An address record is:
1. a type byte `t`, built from flags:
   - `T_INDEX` (1<<1): then the index register and the scale, one byte each;
   - `T_OFFSET` (1<<2): then a 4-byte offset, followed by 4 more bytes if `T_64` (1<<6) is also set;
   - `T_SYM` (1<<4): then a 1-byte symbol slot;
   - `T_FCONST` (1<<3): then an 8-byte IEEE double, in two 4-byte halves;
   - `T_SCONST` (1<<5): then 8 bytes (`NSNAME`) of string;
   - `T_TYPE` (1<<0): then the operand type byte, `D_AX`…`D_R15`, `D_X0`…, `D_EXTERN` 113, `D_STATIC` 114, `D_AUTO` 115, `D_PARAM` 116, `D_CONST` 117, `D_ADDR` 120 and so on, with `D_INDIR` added for memory operands.

**Symbols** go into a cache of 50 slots (`NSYM`). The first use of a symbol with a given type writes:
- an `ANAME` record with the slot, the type and the NUL-terminated name;
- or, with `-T`, an `ASIGNAME` record that also carries the 4-byte signature.

Later uses refer to the slot. When the cache wraps, slots are reused, and the reader follows the same rule. rucc's writer must follow it exactly, because `6l`'s reader (`6l/obj.c`) does.

**File and line history** comes from `AHISTORY` records. `outhist` writes one per `#include` push and pop, and one per `#line`. The file name is split on `/` into `D_FILE` name records, `<` marks the root, and the line numbers are the cumulative "absolute line" kencc uses. `#pragma lib` also becomes a history record, with offset -1, and that is how `6l` learns which libraries to load (06.5). The debug line table (`pc/line`) that `6l` writes is computed from these records, so `acid`, `db` and `snap` show the right file and line only if rucc's history is right.

**Data** is `ADATA` records:
- `from` is the symbol and its offset;
- `from.scale` is the width;
- `to` is the constant, the address or the string bytes, at most 8 per record.

**Globals** are `AGLOBL` records with the size in `to.offset`. **Functions** are `ATEXT` records: `from` is the symbol and `to.offset` is the frame size. `from.scale` holds the flags:
- `NOPROF` 1;
- `DUPOK` 2.

These are the only two (`6.out.h:3-4`). The assembler writes them as `TEXT f(SB), 1, $0`. Plan 9 has no split stacks, so there is no stack-check flag.

The file ends with an `AEND` record.

**The `.7` format** (`cmd/7c/swt.c`, `7.out.h` with 377 opcodes) has the same structure. The address records carry arm64 operand kinds: shifted and extended registers, pre/post-indexed addresses, condition codes, register pairs and the `D_SP` and `D_ZR` distinctions. The fields differ, the framing does not.

**Libraries.** `ar` builds `.a` files with a `__.SYMDEF` member computed by `libmach`'s object reader (`libmach/6obj.c`, `7obj.c`). `nm`, `size`, `ar` and `acid -l` all read `.6` and `.7` through `libmach`. rucc's output must therefore be readable by `libmach` as well as by `6l`. That is the same format, read by a second reader, and the tests run both (07.6).

## 07.2 What rucc must emit: a `Prog` layer

rucc's back ends today select machine instructions and encode them (`rucc-codegen` with `rucc-asm`). For Plan 9 targets, the last stage changes:
1. **Instruction selection** produces rucc's machine instructions as before, restricted to the subset the Plan 9 linker encodes (07.3).
2. **A new lowering** maps each machine instruction to a `Prog` with Plan 9 operands:
   - `MOVQ`, `MOVL`, `MOVW`, `MOVB` for the four widths;
   - `LEAQ`;
   - operand order "source, destination";
   - Plan 9's `SB`, `SP`, `FP` addressing.

   Static data becomes `ADATA` and `AGLOBL`. Functions become `ATEXT`. The frame size is the frame rucc laid out, minus the return address.
3. **The writer** of 07.1 serializes the `Prog` list. It goes in a new module, `rucc-object/src/plan9.rs`.

There is no encoder at the end. Branch displacements, long and short jumps, and alignment padding are `6l`'s job. rucc emits `AJMP` and the conditional jumps (`AJEQ`…) with a `D_BRANCH` target that is the index of a `Prog`.

**Prologues and epilogues belong to the linker.** `6l`'s `dostkoff` (`6l/pass.c`) inserts the `SUBQ $framesize, SP` after `TEXT`, and an `ADDQ` before each `RET`. rucc must therefore emit functions without them. It states the frame size in `ATEXT` and addresses locals as offsets from the adjusted `SP` (`D_AUTO`, negative offsets from the frame top) and parameters from `FP` (`D_PARAM`). `7l` does the same for arm64, with its own stack and link-register handling. In arm64 kencc frames, `R30` (the link register) is saved by the linker-inserted prologue for non-leaf functions. rucc tells `7l` a function is a leaf the same way `7c` does: a frame size of 0 with no `BL` inside.

**Literal pools and large constants.** `7l` places literal pools itself. `6l` handles 64-bit immediates by choosing `MOVQ $imm64` encodings. rucc emits constants as operands and does not build its own pools.

## 07.3 The `optab` contract

`6l/optab.c` (1,209 lines) and `7l/optab.c` (478 lines) list every opcode and operand-kind combination the linkers can encode. A `Prog` outside the table is a link error: "doasm: notfound". rucc's `Prog` lowering is generated from a machine-readable copy of the two tables, made by `r9 optab --extract` from the pin and checked into `rucc` as `rucc-codegen/src/plan9/optab_{amd64,arm64}.rs`. Each lowering rule is checked against the table at rucc's build time.

This decides what rucc can generate:
- **amd64.** `6l` knows the base x86-64 integer set with `CMOVcc`, SSE and SSE2, the `x87` instructions, and the system instructions the kernel needs. Probed at the pin, `6l/optab.c` has no entry for `PSHUFB` (SSSE3), `POPCNT`, `LZCNT`, `CRC32`, `AESENC`, `PCLMULQDQ`, `MOVBE`, `RDRAND`, `XSAVE`, `MOVNTDQ` or any AVX form (`VPXOR`, `VMOVDQU`). The tree's assembly writes those as `BYTE` sequences. rucc's Plan 9 amd64 target is therefore baseline x86-64 with `CMOV` and SSE2, and nothing more. If a later 9front `6l` gains instructions, the extracted table grows with the pin.
- **arm64.** `7l` knows ARMv8.0 with exclusive-pair atomics (`LDAXR`/`STLXR`) and fused multiply-add (`FMADDD`). It has no LSE atomics (`CAS`, `LDADD`, `SWP`), no CRC32, no AES or SHA instructions, no SVE and no FP16. rucc's Plan 9 arm64 target is ARMv8.0, and its atomics lower to exclusive loops.

A rucc optimization that wants an instruction outside the table must not run on Plan 9 targets. The table is the target's instruction set as far as rucc is concerned.

## 07.4 Type signatures

With `-T` (on in every build, through `CFLAGS=-FTVw`), each external symbol's first `ANAME` is an `ASIGNAME` with a 32-bit signature. `6l` checks that every object agrees (`6l/obj.c:833`): "incompatible type signatures %lux(%s) and %lux(%s) for %s".

The signature is `signat` (`cc/dcl.c:1099`), a hash over the type chain:
- it starts at `s = 0`;
- for each link, `s = s*thash1 + thash[etype]`;
- arrays add `s*thash2 + 0`;
- functions fold each parameter's signature with `thash3`;
- structs and unions fold their members, and use a back-reference index `sigind` for a struct already seen in the same signature, so recursive types terminate;
- incomplete types stop at the incomplete one;
- the hash includes neither names nor widths.

`thash`, `thash1`, `thash2` and `thash3` are constants defined in `cc/dcl.c`. rucc copies them as data with the source line cited.

rucc's implementation is a function over `rucc-types` that walks the same structure, with `sametype` replaced by rucc's own type equality under kencc's rules (06.7). It is tested by `rucc-compat/plan9/signof/`: 2,000 types extracted from the tree, each compiled by G's `6c` as `signof(T)` into a data word and by rucc. The two must agree on every case. A difference means R and G objects cannot link together, and that breaks mixed bisection (12.8), so P3 gates on it.

`6l` also gives some symbols a fixed signature: `zerosig` for `etext`, `edata`, `end` and the export table. rucc emits plain `ANAME` for those, as `6c` does.

## 07.5 The assembler: `6a` and `7a` syntax

The tree has 35,898 lines of `.s`:
- the kernels' `l.s`, `apbootstrap.s` and `rebootcode.s`;
- `libc`'s string, atomic, `setjmp` and `main9` routines;
- `libsec`'s crypto kernels;
- `libmp`'s bignum loops;
- the boot programs.

They are built by `$AS`, which is `6a` or `7a`. Until P6 they are built by the tree's own assemblers and counted `ref-as`. From P6, `r9-build` also puts a copy of `r9-cc` over `6a` and `7a`, and `r9-cc` sends these calls to rucc through `r9 ccd`. rucc reads the command line of the assembler with its `plan9-as` driver.

rucc's Plan 9 assembler front end goes in `rucc-asm/src/plan9/{amd64,arm64}.rs`. It reads:
- **Instructions**: `MOVQ $1, AX` and the rest, with operands as source then destination, and size suffixes;
- **Pseudo-registers**: `SB` (static base), `FP` (arguments), `SP` (locals), `PC`;
- **Pseudo-instructions**: `TEXT sym(SB), flags, $frame`, `DATA sym+off(SB)/width, value`, `GLOBL sym(SB), flags, $size`;
- **Raw bytes**: `BYTE`, `WORD`, `LONG`, `QUAD`;
- **The preprocessor**: `#include` and `#define`. The assemblers use the same built-in preprocessor as the compilers (06.4).
- **Register names**: `R8`–`R15` and `X0`–`X15` on amd64; `R0`–`R30`, `ZR`, `F0`–`F31`, `V0`–`V31` and arm64's `MOVP` pairs on arm64.

Its output is the same `Prog` list as the compiler's, written by 07.1's writer. The test is byte-identical `.6` and `.7` files against `6a` and `7a` for every `.s` in the tree. That is 440 files, and it is a stronger test than the C side can have, because the assembler makes no choices.

`-S` output from rucc is in this syntax, and it reassembles with `6a` to the same object. That gives a third way to compare R with G: `r9 sdiff unit.c` prints rucc's and `6c`'s `-S` side by side.

## 07.6 Object diffs

`r9 objdiff a.6 b.6` decodes both files into the `Prog` list, symbol by symbol, and prints the differences in `6c -S` form. It is used for:
- **the fixed point** (12.4), where R-built `6c` and G-built `6c` must write byte-identical `.6` files for every unit;
- **the assembler** (07.5), where outputs must be byte-identical;
- **R against G on C units,** where differences are expected and the tool is for reading, not grading.

`r9 objcheck file.6` runs three readers over one object:
- `libmach`'s reader, through `nm`;
- `r9`'s own decoder;
- `6l`'s reader, through a link of the object's whole library or program as the build links it.

The first two must agree on the symbol list, and the link must succeed with the same symbol table `nm` shows for G's link. This runs on every R object in P2 and P4.

## 07.7 `acid` output

`-a` writes `acid` declarations for every struct and union, plus every typedef with `-aa`. `-n` writes them to `stem.acid` instead of standard output. The kernel and some libraries use this in their mkfiles to build `acid` libraries (`9/port/portmkfile:15` has a `%.acid` rule, and `9/pc64/mkfile:177` runs `$CC -a -w main.c>acid`). The format is:

```
sizeofQLock = 32;
aggr QLock
{
	Lock 0 use;
	'X' 8 head;
	...
};
defn QLock(addr) { ... }
```

It is generated from the type layout, and the layout is the same as kencc's whenever rucc is correct. rucc's `acid` writer follows `cc/acid.c` and goes in `rucc-object/src/plan9_acid.rs`. The test is byte-identical `.acid` files against G's for every `-a` unit in the tree. This is a useful check on struct layout (`pack`, unnamed members, bit fields), independent of codegen.
