# The Plan 9 C dialect

This document is what `-fkencc` means. It is taken from the compiler at the pin (`/sys/src/cmd/cc`), not from the 1990s paper, wherever the two differ. Each item names:
- the code that decides it;
- its count in the tree;
- what rucc does.

`-fkencc` is implied by every `*-unknown-plan9` target and by `--driver=kencc`. It can also be given alone on a host target. Nothing in the plan needs that, but the dialect must not depend on the Plan 9 target, so `rucc-compat` tests it on `aarch64-apple-darwin` and `x86_64-linux-gnu` as well (15.2).

## 06.1 The language level

kencc is C89 with a short list of C99 and later features. These are its keywords (`cc/lex.c:1150-1188`):

```
auto break case char const continue default do double else enum extern float for goto if
inline int long _Noreturn register restrict return SET short signed signof sizeof static
struct switch typedef typestr union unsigned USED void volatile while
```

There is no `_Bool`, `_Static_assert`, `_Alignas`, `_Atomic`, `_Generic`, `_Thread_local`, `asm`, `__attribute__`, `typeof` or `bool`, so in `-fkencc` mode all of these are ordinary identifiers. The tree uses `true` as a variable (`6c/cgen.c`, `dd.c`). rucc's keyword table is per-mode (`crates/rucc-lex/src/keyword.rs`), and `-fkencc` gets exactly the list above.

C99 features kencc does accept:
- `//` comments;
- declarations after statements;
- `long long`;
- compound literals (`(T){…}`);
- designated initializers in the C99 form.

C99 features it does not accept:
- variable-length arrays;
- flexible array members declared `T a[]`. The tree uses `T a[1]` or `T a[0]`, and rucc's "flexible array member in a struct with no named members" error (145 of the sampled files, through `ape` and `libc` headers) is wrong in this mode;
- `inline` semantics. kencc parses `inline` and ignores it.

In `-fkencc`, rucc rejects what kencc rejects, with an error that names the kencc rule: VLAs, `_Bool`, and `#if` without `-p`. A unit that builds only with rucc is a configuration difference (02.1).

## 06.2 Unnamed members and promotion

A struct or union member may be declared with a type and no name:

```c
typedef struct Ref Ref;
struct Ref { Lock; long ref; };   /* Lock is an unnamed member of type Lock */
```

Member lookup is `dotsearch` (`cc/sub.c:360`). It goes in this order:
1. **By name, at this level.** A member whose name is the identifier. Two such members are an error: "ambiguous structure element".
2. **By type, at this level.** If the identifier is a `typedef` name (or a `typestr`), an unnamed member of that same type matches. This is how `r->Lock` names the unnamed `Lock`.
3. **Recursively, in each unnamed struct or union member, in declaration order.** A match is offset by the member's offset. Two matches at this step are ambiguous.

Steps 1 and 2 return before step 3 runs. So a named member at an outer level hides one of the same name in an unnamed member. That is how `rio`'s `Window` has both `Rune *r` and an unnamed `Frame` with its own `r` (`cmd/rio/dat.h:125`). The same happens in `9/port/portdat.h:73`, 87 and 118. GCC rejects all of these as "duplicate member". kencc's rule is depth-first preference, not "unspecified", whatever the manual says. rucc implements `dotsearch`'s order exactly.

**Promotion** is `typeext` (`cc/sub.c:541`). It applies wherever a value is converted to a type: assignment, argument passing to a prototyped parameter, and return.
- **A struct value** of type S, converted to a struct type T that S contains as an unnamed member at any depth, becomes the member: an inserted `.`.
- **A pointer to S**, converted to a pointer to T, becomes `&p->T`: an added offset.
- **A null pointer constant** converted to any pointer type stays a null pointer. Promotion does not add an offset to a constant 0. This matters for `lock(nil)`-style code paths.

The offset is computed by `dotoffset` (`sub.c:413`), by the same lookup as above.

rucc puts the lookup in `rucc-sema` and inserts the promotion as an explicit `MemberAddr`/`Member` node at conversion time. Codegen never sees an implicit conversion. A pointer conversion between unrelated struct types that is not a promotion is an error in `-fkencc`, as in kencc. There is no `gnu89` warning path (03.2).

**GCC's `-fplan9-extensions`** is a strict subset:
- lookup by typedef name;
- promotion on assignment and argument passing.

Unlike kencc, it has no "outer wins" rule for duplicates: GCC errors on them. rucc's `-fplan9-extensions` gives GCC's behaviour, errors included. `-fkencc` gives kencc's. The two flags share the code and differ in one predicate.

## 06.3 `USED`, `SET`, `signof`, `typestr`

| Form | Meaning | rucc |
|---|---|---|
| `USED(a, b, …)` | a statement. Marks the expressions as used, to silence "set and not used" and "unused parameter" | keyword in `-fkencc`; evaluates nothing; suppresses rucc's equivalent warnings |
| `SET(a, …)` | marks the variables as set, to silence "used and not set" | the same |
| `signof(type)`, `signof(expr)` | an `unsigned long` constant: the type's signature hash (07.4) | constant expression; needs the hash of 07.4 bit for bit |
| `typestr` | a storage-class-like keyword making a typedef whose name is usable in `dotsearch` step 2 | rare; parse, and treat as `typedef` in lookup |

`USED` is a statement, not an expression: `x = USED(y)` is a syntax error in kencc. `va_end(list)` in `u.h` expands to `USED(list)`, so every `va_end` is a `USED`.

## 06.4 The preprocessor

kencc's preprocessor is in `cc/macbody` and `cc/lex.c`. Its directives are:
- `#define`, `#undef`;
- `#include`;
- `#ifdef`, `#ifndef`, `#else`, `#endif`;
- `#line`;
- `#pragma`.

It has no `#if` or `#elif`. With `-p`, the compiler runs `/bin/cpp` (`cmd/cpp`, an ANSI preprocessor) first, and reads its output.

Differences from ANSI that rucc implements in `-fkencc` without `-p`:
- **Macro parameters are substituted inside string and character literals** in the macro body. `#define STR(x) "x is x"` with `STR(hi)` gives `"hi is hi"`.
- **No predefined macros** at all. That includes `__STDC__`, `__FILE__` and `__LINE__`. They are absent unless `-p`, and then `cpp` defines `__STDC__=1` only when `pcc` asks.
- **Include search order.** For `"file"`: the directory of the including file (removed by `-.`), then each `-I` in order, then `/sys/include`, then `/$objtype/include`. For `<file>`: the same without the first step.
- **`#include` accepts only a literal** `"file"` or `<file>`, never a macro name. rucc gives kencc's error.
- **`#ifdef` and `#ifndef` take one identifier.** Trailing tokens are an error.
- **Unknown directives are errors:** `unknown #: name` (`macbody:188`). The directive table is `mactab` (`macbody:155`).
- **`#` in a macro body is literal.** `macdef` escapes it and `macexpand` restores it, so `#x` and `a##b` are plain text with the parameter substituted.

With `-p`, rucc behaves as `cpp` piped into kencc: ANSI preprocessing, then the kencc front end. `cmd/cpp` is a reasonably standard ANSI preprocessor, and rucc's own `rucc-pp` is used in ANSI mode. Where `cmd/cpp` differs from ANSI, rucc follows `cmd/cpp`. The differences are found by running both over the 11 `-p` directories in P1, and listed in `rucc-compat/plan9/cpp/`.

`pcc` runs `cpp` itself and feeds `6c` on stdin. On that path rucc must accept `#line` in `cpp`'s exact form (`#line 12 "file.c"`) and use it for `AHISTORY` records (07.1).

## 06.5 Pragmas

`macprag` (`cc/macbody`) acts on six pragmas and ignores every other one silently:

| Pragma | Count | Effect | rucc |
|---|---|---|---|
| `lib "libx.a"` | 100 | recorded as a history entry with offset -1, so it reaches the object, and `6l` adds the library to the link | emit the same `AHISTORY` record (07.1) |
| `pack on\|off\|N` | 50 | struct layout: `packflg` sets member alignment to 1 (on) or to the natural value (off) | layout rule in `-fkencc` (08.1) |
| `fpround` | rare | floating-point rounding of intermediates on 386 | accept; act on 386 only |
| `profile on\|off` | 28 | turns off the profiling prologue for following functions | record per function; `-p` profiling output is P10 |
| `varargck argpos\|type\|flag` | 554 | format checking with `-F` (06.6) | implement |
| `incomplete struct S` | 110 | suppresses the "incomplete type" warning for S in `acid` output | record; affects only `-a` output and warnings |

`#pragma once` (24), `src` (54), `interface` (26), `warning` (10), `ref`, `implementation`, `unused`, `export`, `optimize`, `message` and `argsused` are ignored by kencc. rucc must ignore them too. In particular, rucc must **not** honour `#pragma once` in `-fkencc` mode, because kencc includes such a header again. Headers in the tree survive double inclusion because kencc accepts identical redeclarations (06.7). rucc must accept the same redeclarations. The census reports any header included twice in one unit so the case is tested.

## 06.6 Format checking

`-F` turns on `print`-format checking (`cc/dpchk.c`). The table is built from `#pragma varargck` lines:
- **`argpos f n`:** function `f` has its format at argument `n`, and the variadic arguments follow.
- **`type "verb" T`:** verb `%verb` takes type `T`. Verbs may carry flags (`"lld"`), and `T` may be a pointer.
- **`flag 'c'`:** `c` is a flag character.

The library headers declare all of `print`'s verbs, and `fmtinstall` verbs are declared beside each `fmtinstall` (`%r`, `%C`, `%S`, `%H`, `%I`, `%E`, `%V`, …). A mismatch is a warning. rucc ports `dpchk.c`'s algorithm, so that the same unit gives the same warnings in the same order (05.8). It is a separate module, `rucc-sema/src/check/kencc_fmt.rs`, sharing nothing with the GCC `format` attribute checker. The two systems differ in what counts as a match: kencc compares the type after promotion with its own rules (06.8).

## 06.7 Declarations kencc accepts

These are all in the tree, and all rejected by rucc today in its C modes:
- **Implicit `int`** at file scope and in function definitions (`flambe.c`, `mk/*.c`).
- **Old-style definitions** mixed with prototypes. A prototype is compatible with an old-style definition when the promoted types agree under kencc's rules, not ANSI's.
- **Enums and `int`.** `enum E` is compatible with `int` in prototypes, definitions and assignments (`draw.h`'s `Drawop`).
- **Qualifiers.** `char*` and `const char*` are compatible in redeclarations (`perror` in APE's `stdio.h` against `libc.h`).
- **Identical redeclarations** of typedefs and structs in the same scope.
- **`extern register`** at file scope (06.9).
- **Arrays of unknown size** at file scope completed later.

## 06.8 Expressions and conversions

**The usual arithmetic conversions** follow `cc/sub.c`'s `tab` (around line 1852), then "at least int" (`arith()`). The result is unsigned if either operand is unsigned at its own width, so `uchar` op `int` is `uint`. rucc implements the table as data, not as a rule, so that every pair is as kencc says. That covers the `vlong` and `float` rows too. `-Wkencc-conversion` counts the places where this differs from ANSI in an observable way (03.9).

**Integer constants.**
- The type of an unsuffixed constant that does not fit in `int` follows `cc/lex.c`'s number reader. Because `long` is 32 bits, it differs from LP64 C on constants between 2^31 and 2^64. P1 transcribes the rule into `rucc-compat/plan9/const/` with one case per boundary, and checks each case against G's `-S` output.
- `L'x'` is a `Rune`, which is `uint`.
- `L"…"` is an array of `Rune`. On 64-bit targets that is `uint[]`, not `wchar_t` as `int`. Three sample files initialise `Rune` arrays from `L""` strings, and rucc rejects them today.

**Shifts.** A right shift of a signed value is arithmetic. Shift counts are not masked by the compiler, and the machine's behaviour is the result. rucc must not treat a large shift as undefined behaviour and fold it away in `-fkencc` mode. It emits the instruction, which is what kencc does. In general, rucc's optimizer runs in `-fkencc` with `-fwrapv`, `-fno-strict-aliasing`, `-fno-delete-null-pointer-checks` and no undefined-shift reasoning. kencc never exploits undefined behaviour, and 9front's code was written and tested against a compiler that does not. This is P2's most important rule, and it costs a little performance (13).

**Floating point.** `float` arithmetic is done in `float`, not promoted to `double`, unless an operand is `double` (`typeext1` and `allfloat`). This affects rounding in `libmemdraw`'s and the games' float code. rucc's `-fkencc` sets `FLT_EVAL_METHOD 0`, which is already the default on amd64 and arm64. `vlong`-to-float conversions call the runtime helpers on 386 only.

**Compound literals** are `(T){…}` anywhere. **Designators** accept the old forms `[i] v` and `.f v` with no `=`, as well as C99's.

## 06.9 `extern register`

```c
extern register Mach *m;     /* 9/pc64/dat.h */
extern register Proc *up;
```

kencc allocates registers from `REGEXT` downward, in order of declaration within the unit:
- **6c:** `R15`, then `R14`;
- **7c:** `R27`, then `R26`.

`6l` and `7l` know these registers through `REGEXT` and never use them. `-fkencc` reproduces this:
- the variables become register-resident globals;
- the registers are excluded from allocation in every function of every unit compiled with `-fkencc` for that target, whether or not it declares them;
- address-of is an error.

User code has no `extern register`, but the registers are still not allocated, because `6l` reserves them for the whole program. So rucc reserves them for all `*-unknown-plan9` code (08.4).

## 06.10 `-fplan9-extensions` as a GCC flag

`-fplan9-extensions` is tested on `x86_64-linux-gnu` and `aarch64-linux-gnu` against GCC 16 in `rucc-compat/gcc-plan9-ext/`. The cases are GCC's own testsuite cases for the flag (`gcc.dg/anon-struct-11.c` to `-14.c` and related), plus 60 reductions from 9front headers that GCC accepts. Under the parent goal, a difference from GCC here is a GCC-compatibility bug, not a Plan 9 bug.

## 06.11 The command line: rucc as `6c`

`--driver=kencc` makes rucc parse `2c(1)`'s options. rucc is invoked this way by `r9 ccd`, or when it is called as `6c`, `7c`, `8c` or `5c` through a symlink:

| Option | Meaning | rucc |
|---|---|---|
| `-o file` | output name | yes |
| `-D name=def`, `-I dir` | as usual | yes |
| `-F` | format checking | 06.6 |
| `-T` | emit type signatures | 07.4 |
| `-V` | `vlong` warnings: "uses vlong" | yes |
| `-w` | enable warnings | yes |
| `-B` | do not complain about functions without prototypes | yes |
| `-N` | suppress the optimizer | rucc's `-O0`, plus no registerization |
| `-S` | print the assembly on stdout | rucc prints Plan 9 assembler syntax (07.5) |
| `-a`, `-aa`, `-n` | `acid` declarations to stdout, or with `-n` to `stem.acid` | 07.7 |
| `-p` | run `cpp` first | 06.4 |
| `-.` | no own-directory include lookup | 06.4 |
| `-c` | in `pcc`'s calls: compile only | yes |
| `-l`, `-d…`, `-m`, `-s` | debugging and rarely used options | accepted; `-d…` ignored with a note |

When no file is given, the source is read from stdin. When there are several files, each is compiled to its own `.6` (kencc compiles a list of files). An optimization level is not a kencc option. rucc in `--driver=kencc` mode runs at the level set by `R9_OPT` in the shim's environment: `2` by default, and `0` for `-N`. `R9_OPT` is part of the rig, recorded in every record, and the same for every unit in a graded run.
