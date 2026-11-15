# 9h monitor

This is the monitor of `9h`, the hosted runtime of plan 10. It runs a 9front kernel on the hypervisor of the host. The plan is in `docs/plan/10-the-hosted-runtime-9h.md`.

| Crate | What |
|---|---|
| `h9-vm` | the guest RAM, the kernel loader (a.out and U-Boot images), the device tree builder and the exits |
| `h9-dev` | the bus, the PSCI calls and the bring-up devices (PL011 and PL031) |
| `h9-hvf` | the Hypervisor.framework back end, for macOS on arm64 |
| `h9` | the `9h` command |

The KVM back end, the hypercall devices and the `9h` kernel come in later steps of P7.

## Build

`./build.sh` builds the release binary. On macOS it signs the binary with the entitlement `com.apple.security.hypervisor`, because the framework refuses a process without it.

## Run

This runs the `qemu` kernel of the tree, with no change, on macOS 15 or later on arm64:

```
target/release/9h run --kernel 9qemu.u --ini plan9.ini --mem 512
```

`plan9.ini` needs `console=0`. At the `bootargs` prompt, type `!rc` for a shell. `--timeout SECS` ends the session after SECS seconds. `--strict` stops the session at an access to an address that has no device. `H9_TRACE=1` logs each exit of the vCPU.

## Test

`cargo test` runs the unit tests on each host. The tests do not need a hypervisor.
