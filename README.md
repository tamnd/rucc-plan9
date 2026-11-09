# rucc-plan9

A harness that builds pinned, unmodified 9front trees with [rucc](https://github.com/tamnd/rucc) and with the tree's own compilers. It boots both builds under QEMU, runs the tree's own tests against them, and records the results in committed files.

The repository also contains `9h`. `9h` runs an unmodified 9front kernel in a hardware virtual machine on macOS (Hypervisor.framework) and on Linux (KVM). It shows the 9front screen in a host window, so the GUI programs of 9front run on the host desktop with no change.

The plan is in [docs/plan](docs/plan). It has twelve milestones, P0 to P11. Each milestone has a tracking issue and a GitHub milestone with the same name. We tick the checklist in the issue when a pull request lands.

## The rules

**No compiler code.** This repository does not compile C for a graded build. Every compile goes to rucc or to the tree's own compiler through the `r9-cc` shim. The shim records the call and does not change it. A fix to rucc goes to [tamnd/rucc](https://github.com/tamnd/rucc).

**No source patches and no copied 9front files.** We fetch each tree by commit hash and each release image by SHA-256. The harness does not write into a tree. A build writes into a guest overlay, and we discard the overlay after the build.

**No reductions.** A reduced test case goes to [rucc-corpus](https://github.com/tamnd/rucc-corpus), or to [rucc-compat](https://github.com/tamnd/rucc-compat) when it compares two compilers. This repository links to it by issue number.

**No hidden configuration.** Every input that changes a graded build is in a committed file: pins, rows, kernel configurations, guest scripts, GUI scripts, masks and exclusions.

## R and G

R is a build of a tree with rucc in the compiler slot. G is a build of the same tree by the tree's own compilers (`6c`, `7c`, `6a`, `7a`), which are built by themselves. A test case counts only if G passes it in all three of its baseline runs.

## Milestones

| | Scope |
|---|---|
| P0 | the instrument: pins, guests, the `r9-cc` shim, G baselines, census and demands |
| P1 | rucc speaks Plan 9: targets, the kencc driver, the dialect, the preprocessor, 9P |
| P2 | the amd64 ABI and the `.6` object writer |
| P3 | the amd64 world: all of user space built by rucc |
| P4 | arm64: the `.7` object writer and the arm64 world |
| P5 | the kernels: `pc64` and the arm64 `qemu` kernel built by rucc boot and pass |
| P6 | the assembler: `6a` and `7a` syntax |
| P7 | `9h` core: the kernel files and the host monitor, headless |
| P8 | the GUI on macOS and Linux |
| P9 | the performance gates |
| P10 | every version, steady state and 386 |
| P11 | arm, and the rootless decision |

## License

Apache License 2.0. See [LICENSE](LICENSE). The `9h` kernel files under `9h/arm64` and `9h/amd64` use the MIT license, as 9front does.
