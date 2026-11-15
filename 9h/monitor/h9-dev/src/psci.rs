//! The calls of the Power State Coordination Interface that the arm64 kernel makes with `HVC`
//! (`main.c`: `CPU_ON` in `mpinit`, `CPU_OFF` and `SYSTEM_RESET` in `exit`). The 9h hypercalls
//! use the same path, with numbers in the range of the hypervisor vendor (plan 10.5).

/// `PSCI_VERSION`.
pub const VERSION: u32 = 0x8400_0000;
/// `CPU_OFF`.
pub const CPU_OFF: u32 = 0x8400_0002;
/// `CPU_ON`, 32-bit. The 64-bit number is this one with `SMC64` set.
pub const CPU_ON: u32 = 0x8400_0003;
/// `AFFINITY_INFO`, 32-bit. The 64-bit number is this one with `SMC64` set.
pub const AFFINITY_INFO: u32 = 0x8400_0004;
/// The bit of the function number that selects the 64-bit calling convention.
pub const SMC64: u32 = 0x4000_0000;
/// `MIGRATE_INFO_TYPE`.
pub const MIGRATE_INFO_TYPE: u32 = 0x8400_0006;
/// `SYSTEM_OFF`.
pub const SYSTEM_OFF: u32 = 0x8400_0008;
/// `SYSTEM_RESET`.
pub const SYSTEM_RESET: u32 = 0x8400_0009;
/// `PSCI_FEATURES`.
pub const FEATURES: u32 = 0x8400_000a;

/// The `SUCCESS` return value.
pub const SUCCESS: u64 = 0;
/// The `NOT_SUPPORTED` return value, -1.
pub const NOT_SUPPORTED: u64 = u64::MAX;
/// The `INVALID_PARAMETERS` return value, -2.
pub const INVALID_PARAMETERS: u64 = u64::MAX - 1;
/// The `ALREADY_ON` return value, -4.
pub const ALREADY_ON: u64 = u64::MAX - 3;
/// The `ON_PENDING` return value of `CPU_ON`, -5.
pub const ON_PENDING: u64 = u64::MAX - 4;

/// What the monitor does for a call.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Put this value in X0 and go on.
    Return(u64),
    /// Stop this vCPU.
    CpuOff,
    /// Start the vCPU with this affinity at `entry`, with `context` in X0.
    CpuOn {
        /// The affinity of the target, as in MPIDR.
        target: u64,
        /// The guest physical address to start at.
        entry: u64,
        /// The value of X0 at the start.
        context: u64,
    },
    /// Return the state of the vCPU with this affinity: 0 when it runs, 1 when it is off, 2 when
    /// it is starting.
    AffinityInfo {
        /// The affinity of the target, as in MPIDR.
        target: u64,
    },
    /// End the session.
    Off,
    /// Start the session again.
    Reset,
}

/// The action for a call with the registers X0 to X5. None when the number is not a PSCI call.
///
/// `CPU_ON` and `AFFINITY_INFO` have a 32-bit and a 64-bit number. The kernel of the pinned tree
/// uses the 64-bit `CPU_ON`, and older 9front kernels use the 32-bit one, so both are known. In
/// a 32-bit call, the arguments are the low 32 bits of the registers.
#[must_use]
pub fn call(x: &[u64; 6]) -> Option<Action> {
    let f = x[0] as u32;
    if f & 0xbfff_ffe0 != 0x8400_0000 {
        return None;
    }
    let base = f & !SMC64;
    let wide = f & SMC64 != 0;
    if wide && base != CPU_ON && base != AFFINITY_INFO {
        return Some(Action::Return(NOT_SUPPORTED));
    }
    let x = if wide { *x } else { x.map(|r| r & 0xffff_ffff) };
    Some(match base {
        VERSION => Action::Return(0x1_0000),
        CPU_OFF => Action::CpuOff,
        CPU_ON => Action::CpuOn {
            target: x[1],
            entry: x[2],
            context: x[3],
        },
        AFFINITY_INFO if x[2] == 0 => Action::AffinityInfo { target: x[1] },
        AFFINITY_INFO => Action::Return(INVALID_PARAMETERS),
        MIGRATE_INFO_TYPE => Action::Return(2),
        SYSTEM_OFF => Action::Off,
        SYSTEM_RESET => Action::Reset,
        FEATURES => {
            let known = [
                VERSION,
                CPU_OFF,
                CPU_ON,
                CPU_ON | SMC64,
                AFFINITY_INFO,
                AFFINITY_INFO | SMC64,
                MIGRATE_INFO_TYPE,
                SYSTEM_OFF,
                SYSTEM_RESET,
                FEATURES,
            ];
            Action::Return(if known.contains(&(x[1] as u32)) {
                0
            } else {
                NOT_SUPPORTED
            })
        }
        _ => Action::Return(NOT_SUPPORTED),
    })
}

#[cfg(test)]
mod tests {
    use super::{Action, call};

    #[test]
    fn the_calls_of_the_kernel_are_known() {
        assert_eq!(
            call(&[0xC400_0003, 1, 0x4010_0000, 1, 0, 0]),
            Some(Action::CpuOn {
                target: 1,
                entry: 0x4010_0000,
                context: 1
            })
        );
        assert_eq!(
            call(&[0x8400_0003, 0xffff_0000_0000_0002, 0x4010_0000, 2, 0, 0]),
            Some(Action::CpuOn {
                target: 2,
                entry: 0x4010_0000,
                context: 2
            }),
            "a 32-bit call uses the low 32 bits of each register"
        );
        assert_eq!(
            call(&[0xc400_0002, 0, 0, 0, 0, 0]),
            Some(Action::Return(super::NOT_SUPPORTED)),
            "CPU_OFF has no 64-bit number"
        );
        assert_eq!(call(&[0x8400_0009, 0, 0, 0, 0, 0]), Some(Action::Reset));
        assert_eq!(call(&[0x8400_0002, 0, 0, 0, 0, 0]), Some(Action::CpuOff));
        assert_eq!(
            call(&[0x8400_0000, 0, 0, 0, 0, 0]),
            Some(Action::Return(0x1_0000))
        );
        assert_eq!(
            call(&[0x8400_000a, 0xC400_0003, 0, 0, 0, 0]),
            Some(Action::Return(0))
        );
        assert_eq!(
            call(&[0x8400_000a, 0x8400_0015, 0, 0, 0, 0]),
            Some(Action::Return(super::NOT_SUPPORTED))
        );
        assert_eq!(
            call(&[0xC400_0004, 3, 0, 0, 0, 0]),
            Some(Action::AffinityInfo { target: 3 })
        );
        assert_eq!(call(&[0xC600_0000, 0, 0, 0, 0, 0]), None);
        assert_eq!(super::ALREADY_ON.cast_signed(), -4);
        assert_eq!(super::ON_PENDING.cast_signed(), -5);
    }
}
