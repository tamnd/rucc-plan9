# Sibling repositories

The family rule applies here too:
- fixes go to `rucc`;
- tools go to `rucc-plan9`;
- reductions go to the corpora.

This document lists what each repository gains, at which milestone of document 16, and what this folder learns from `rucc-bsd` and `rucc-kernel`.

## 15.1 `tamnd/rucc`

All compiler work in documents 03 and 06 to 08.

| Area | Issues to open at P0 | Milestone |
|---|---|---|
| Plan 9 targets (03.1) | `Os::Plan9` in `rucc-tuple` and `rucc-target`; `x86_64-unknown-plan9` and `aarch64-unknown-plan9`; the data model (`long` 32 bits, 08.1); no predefined macros at all; tier 4 in `docs/TARGETS.md`, moving to tier 2 at P5 | P1 |
| `--driver=kencc` (06.11) | kencc's command line exactly: `-FTVw`, `-I`, `-D`, `-.`, `-p`, `-a`/`-aa`, `-n`, `-o`, stdin input, `R9_OPT` | P1 |
| `-fkencc` (06) | the dialect: keywords, unnamed members and promotion (`dotsearch`, `typeext`), `USED`/`SET`/`signof`/`typestr`, lax declarations, unsigned-preserving conversions | P1, P2 |
| kencc preprocessor (06.4) | the built-in preprocessor's rules: no stringify or paste, arguments substituted in strings, `unknown #` errors, literal-only `#include` | P1 |
| pragmas (06.5) | `lib`, `pack`, `fpround`, `profile`, `varargck`, `incomplete`; `once` ignored, as kencc ignores it | P1, P3 |
| `-fplan9-extensions` (06.10) | GCC 16's option, same semantics, for host targets | P1 |
| brace elision (03.2) | the C bug the probe found; it affects every target | P1 |
| `rucc-9p` (03.7) | a 9P2000 client crate, and a sysroot provider over it with a `qid` cache | P1 |
| `Convention::Kencc` (08.2) | the convention for both architectures: first argument in `RARG`, the rest on the stack, every register caller-saved, struct return by hidden pointer | P2 (amd64), P4 (arm64) |
| `Layout::Kencc` (08.1) | struct and union padding to 8, struct members aligned to 8, bit fields by `tfield`, 8-byte parameter slots | P2 |
| `extern register` (06.9) | global register variables, with the register reserved in every function of the unit (`R14`/`R15`, `R26`/`R27`) | P2 |
| the Plan 9 object writer (07.1, 07.2) | `rucc-object/src/plan9.rs`: the `Prog` layer, `.6` and `.7` records, `ANAME`, `AHISTORY`, `#pragma lib`, the symbol cache | P2 (`.6`), P4 (`.7`) |
| `optab` conformance (07.3) | instruction selection limited to what `6l`/`7l` encode, from the extracted tables | P2, P4 |
| signatures (07.4) | `signat` with `thash` and `sigind`, bit-exact with kencc's | P3 |
| `varargck` format checking (06.6) | `kencc_fmt.rs` | P3 |
| acid output (07.7) | `-a`, `-aa` and `-n`, so layouts can be compared and the kernel's `.acid` files build | P3 |
| kernel mode (09.3) | no SIMD in integer code on Plan 9 targets, decided by open question 8 | P5 |
| the Plan 9 assembler (07.5) | `rucc-asm/src/plan9/`: `6a` and `7a` syntax, byte-identical on the tree's 440 `.s` files | P6 |
| 386 and arm (16, P11) | `.8` and `.5` writers, `8c` and `5c` conventions | P11 |

Two changes to rucc's own spec follow from this folder, proposed as spec edits at P0:
1. **The target matrix** in `spec/cross-compile/04-target-matrix.md` gains `x86_64-unknown-plan9` and `aarch64-unknown-plan9`. They are the first targets whose object format is not ELF, Mach-O, COFF or Wasm, and the first whose linker is not rucc's or the system's ELF linker. The matrix needs a column for "object format", which today is implied by the OS.
2. **The target ladder** (parent document 14.5) gains a sentence after the BSD one: "and an operating system with its own compiler, ABI and object format: 9front, `tamnd/rucc-plan9`".

## 15.2 `tamnd/rucc-compat`

These are differential corpora. Here the second compiler is kencc where it can run, and GCC 16 where the question is about `-fplan9-extensions`.

| Corpus | Contents | From |
|---|---|---|
| `plan9/abi` | calls between rucc and G objects for every argument and return shape: scalars, `vlong`, `double`, structs of 1 to 64 bytes, variadic, function pointers. The two are linked by `6l`/`7l`, and the program runs in the guest | P2 |
| `plan9/layout` | `sizeof`, `offsetof` and bit-field placement for every struct in `/sys/include` and `/$objtype/include`, compared through acid output (07.7) | P2 |
| `plan9/signof` | `signof` of every type in `/sys/include`, against kencc's | P3 |
| `plan9/const` | integer constant typing and the conversion rules, one expression per case, compared by the value printed in the guest (06.8) | P1 |
| `plan9/cpp` | kencc preprocessor cases: strings with arguments, `#` in bodies, unknown directives, `#line` from `pcc` | P1 |
| `plan9/asm` | every `.s` file in the tree, assembled by `6a`/`7a` and by rucc, compared byte for byte. It is stored as a manifest of paths at the pin, not as copies | P6 |
| `gcc-plan9-ext` | `-fplan9-extensions` cases on host targets, rucc against GCC 16 (06.10) | P1 |

## 15.3 `tamnd/rucc-corpus`

Reduced tests, each a small program with an expected result. Each new facet is motivated by a named file in the tree, cited in a comment and never copied:

| Facet | Tests | From |
|---|---|---|
| `kencc-members` | outer-level names winning over unnamed-member names (`rio`'s `Window` and `Frame`, `portdat.h`); promotion of a struct value and a pointer to an unnamed member's type; "ambiguous structure element" | P1 |
| `kencc-conversions` | `uchar op int`, `ushort` compares, shifts of unsigned-preserving results | P2 |
| `kencc-layout` | one-char structs of size 8; nested structs aligned to 8; bit fields in `int` on amd64 and in `long` on arm64 | P2 |
| `kencc-regs` | `extern register` across calls, in signal (`notify`) handlers, and in functions that call `setjmp` | P2 |
| `kencc-varargck` | each `%` verb registered in the tree, with the warnings kencc gives | P3 |
| `kencc-perf` | performance cases from 13.3, each a function where R was slower than G, with the bound | P9 |
| `brace-elision` | the C bug of 03.2, on every target | P1 |

## 15.4 `tamnd/rucc-harness`

`rucc-bsd` created `rucc-harness` at its B0 with four crates: `harness-shim`, `harness-agent`, `harness-stats` and `harness-elf`. `rucc-plan9` uses three of them:
- **`harness-shim`.** It provides the `compile.jsonl` record. `r9-cc` writes the same schema from Plan 9 C, and `r9 ccd` writes rucc's side with the Rust crate. Two fields are added, both optional, so the other harnesses are unaffected:
  - `origin` gains the values `ref-as` and `other-arch`;
  - `ninep_wait_ms` records the 9P wait.
- **`harness-agent`**, for jobs on gpc, the servers and the M4.
- **`harness-stats`**, for 13's medians and ratios.

`harness-elf` does not apply, because Plan 9 objects are not ELF. `r9-obj` is the equivalent. It stays in `rucc-plan9` until another harness needs it.

## 15.5 `tamnd/rucc-cross`

zig has a `plan9` OS tag and its own Plan 9 `a.out` writer. Its facts are used as a cross-check, not as an authority:
- the `a.out` header;
- the system call numbers;
- the `Tos` layout.

At P1, any disagreement between zig's facts and the pin's headers is resolved in favour of the pin and reported to zig. There are no sysroots to make: the guest is the sysroot (05.4).

## 15.6 What this folder learns from `rucc-bsd`

- **The shim and the census.** These are `rb-cc` and `rb demands`, almost unchanged. The difference is the transport: Plan 9 cannot run rucc, so the shim is a client and rucc runs on the host (05.3).
- **G built from the same commit by the reference compiler.** For the BSDs, G is the pinned GCC. Here it is kencc built by itself, which is a stricter oracle, because kencc's output is deterministic and the fixed point (12.4) is available.
- **No knob turned to dodge rucc.** The Plan 9 equivalent is the namespace. `r9-build` binds the compiler slot and nothing else (05.3), and `mk kernels` builds every configuration (09.1). The census states the scope, instead of a smaller build.
- **Baselines with three runs and flake rules** (12.1), from `rb`'s `baselines/`.
- **Reports with the census on the first line** (14.7).

## 15.7 What this folder learns from `rucc-kernel`

- **No SIMD in kernel code.** `rucc-kernel` needs general-registers-only code (kernel 08.3), and rucc has the mode (`rucc-session`). Plan 9's kernel mode (09.3) reuses it.
- **Fixed registers.** rucc has no `-ffixed-<reg>` at 0.30.15. `extern register` (06.9) needs the same mechanism, so P2 builds it once, as a register reservation any target can request. The Linux kernel's `-ffixed-x18` on arm64 then uses it too.
- **Assembly is part of the build.** `rucc-kernel` made rucc's assembler handle the kernel's macro assembly before claiming the kernel. Here, the tree's assemblers build the `.s` files until P6, and the census marks them `ref-as`. The claim says so until P6 closes it.
- **Object bisection.** `rucc-kernel`'s mixed builds, which link R and G objects, are the model for `r9 mix` (12.8). They are simpler here, because kencc objects carry no compiler-specific sections.
- **Performance method.** The interleaved boots, medians and coefficient-of-variation rule of 13.3 come from `rucc-kernel` 12.

What flows back:
- **A non-ELF object layer.** The `Prog` layer (07.2) is rucc's first output that is not a relocatable object in a standard format. It is a model for any later target with its own linker.
- **The brace-elision fix** (03.2), which affects every target.
- **The `-fplan9-extensions` option**, which Linux code does not use, but which GCC compatibility requires.
