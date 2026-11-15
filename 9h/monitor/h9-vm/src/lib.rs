//! The parts of the 9h machine that are the same for each hypervisor (plan 10.3 and 10.8): the
//! RAM of the guest, the kernel image, the device tree, and the exits of a vCPU.

pub mod fdt;
pub mod kernel;
pub mod ram;

pub use ram::Ram;

/// Why a vCPU stopped, after the back end decoded the exit of its hypervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// A load from a guest physical address that is not RAM.
    MmioRead {
        /// The guest physical address.
        pa: u64,
        /// The size of the access in bytes: 1, 2, 4 or 8.
        size: u8,
        /// The register that gets the value. 31 is the zero register.
        reg: u8,
        /// The value is sign-extended.
        signed: bool,
        /// The register is 64-bit, not 32-bit.
        wide: bool,
    },
    /// A store to a guest physical address that is not RAM.
    MmioWrite {
        /// The guest physical address.
        pa: u64,
        /// The size of the access in bytes.
        size: u8,
        /// The value.
        value: u64,
    },
    /// A hypercall, `HVC #0` on arm64. The arguments are X0 to X5.
    Hypercall([u64; 6]),
    /// An access to a system register that the hypervisor traps.
    SysReg {
        /// A read (`MRS`), not a write.
        read: bool,
        /// The general register of the access.
        rt: u8,
        /// The syndrome without the register and the direction, which names the register.
        iss: u64,
    },
    /// The vCPU waits for an interrupt (`WFI`).
    Wait,
    /// The virtual timer fired.
    Timer,
    /// The monitor asked the vCPU to stop running.
    Kicked,
    /// Anything else. The text says what it was.
    Other(&'static str, u64),
}
